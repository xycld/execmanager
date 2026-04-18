use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GovernancePlatform {
    Linux,
    MacOs,
    Unsupported,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GovernanceCapability {
    FullyEnforced,
    PartiallyEnforced,
    ObservableOnly,
    Unavailable,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum PlacementState {
    Applied { target: String },
    Attempted { reason: String },
    NotApplicable { reason: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResourceProfile {
    pub memory_max_bytes: Option<u64>,
    pub cpu_max_micros: Option<u64>,
    pub cpu_period_micros: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResourceMetrics {
    pub memory_current_bytes: Option<u64>,
    pub memory_peak_bytes: Option<u64>,
    pub cpu_usage_micros: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EnforcementGap {
    pub scope: String,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GovernanceSnapshot {
    pub platform: GovernancePlatform,
    pub capability: GovernanceCapability,
    pub profile: ResourceProfile,
    pub placement: PlacementState,
    pub current: ResourceMetrics,
    pub recent_peak: ResourceMetrics,
    pub enforcement_triggered: bool,
    pub enforcement_gaps: Vec<EnforcementGap>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GovernanceRequest {
    pub exec_id: String,
    pub profile: ResourceProfile,
}

impl GovernanceRequest {
    pub fn new(exec_id: impl Into<String>, profile: ResourceProfile) -> Self {
        Self {
            exec_id: exec_id.into(),
            profile,
        }
    }
}

#[derive(Debug, Clone)]
pub struct GovernanceCoordinator {
    environment: GovernanceEnvironment,
}

impl GovernanceCoordinator {
    pub fn for_environment(environment: GovernanceEnvironment) -> Self {
        Self { environment }
    }

    pub fn prepare(&self, request: GovernanceRequest) -> GovernancePlan {
        match self.environment.platform {
            GovernancePlatform::Linux => GovernancePlan::prepare_linux(&self.environment, request),
            GovernancePlatform::MacOs => GovernancePlan::prepare_macos(&self.environment, request),
            GovernancePlatform::Unsupported => GovernancePlan::unsupported(request),
        }
    }
}

#[derive(Debug, Clone)]
pub struct GovernancePlan {
    cgroup_path: Option<PathBuf>,
    state: PlanState,
    snapshot: GovernanceSnapshot,
}

impl GovernancePlan {
    pub fn apply_to_pid(&mut self, pid: u32) -> &GovernanceSnapshot {
        match &mut self.state {
            PlanState::Linux(state) => {
                let procs_path = state.cgroup_path.join("cgroup.procs");
                match fs::write(&procs_path, format!("{pid}\n")) {
                    Ok(()) => {
                        self.snapshot.placement = PlacementState::Applied {
                            target: state.cgroup_path.display().to_string(),
                        };
                        self.snapshot.capability = compute_capability(
                            state.enforcement_requested,
                            true,
                            !self.snapshot.enforcement_gaps.is_empty(),
                            has_observation_path(&state.cgroup_path),
                        );
                    }
                    Err(error) => {
                        self.snapshot.placement = PlacementState::Attempted {
                            reason: format!("failed to place pid {pid} into cgroup.procs: {error}"),
                        };
                        push_gap(
                            &mut self.snapshot.enforcement_gaps,
                            "cgroup_placement",
                            format!("failed to place pid {pid} into cgroup.procs: {error}"),
                        );
                        self.snapshot.capability = compute_capability(
                            state.enforcement_requested,
                            false,
                            true,
                            has_observation_path(&state.cgroup_path),
                        );
                    }
                }
            }
            PlanState::MacOs(state) => {
                if !state.setrlimit_best_effort {
                    self.snapshot.capability = compute_capability(
                        state.enforcement_requested,
                        false,
                        !self.snapshot.enforcement_gaps.is_empty(),
                        state.observation_root.is_some(),
                    );
                }
            }
            PlanState::Unsupported => {}
        }

        &self.snapshot
    }

    pub fn capture(&mut self) -> &GovernanceSnapshot {
        match &self.state {
            PlanState::Linux(state) => {
                self.snapshot.current = read_linux_metrics(&state.cgroup_path);
                self.snapshot.recent_peak = ResourceMetrics {
                    memory_current_bytes: None,
                    memory_peak_bytes: read_u64(state.cgroup_path.join("memory.peak")),
                    cpu_usage_micros: None,
                };
                self.snapshot.enforcement_triggered =
                    linux_enforcement_triggered(&state.cgroup_path);
                self.snapshot.capability = compute_capability(
                    state.enforcement_requested,
                    matches!(self.snapshot.placement, PlacementState::Applied { .. }),
                    !self.snapshot.enforcement_gaps.is_empty(),
                    has_observation_path(&state.cgroup_path),
                );
            }
            PlanState::MacOs(state) => {
                self.snapshot.current = read_observation_metrics(state.observation_root.as_deref());
                self.snapshot.recent_peak = ResourceMetrics {
                    memory_current_bytes: None,
                    memory_peak_bytes: state
                        .observation_root
                        .as_ref()
                        .and_then(|root| read_u64(root.join("memory.peak"))),
                    cpu_usage_micros: None,
                };
                self.snapshot.capability = compute_capability(
                    state.enforcement_requested,
                    state.setrlimit_best_effort,
                    !self.snapshot.enforcement_gaps.is_empty(),
                    state.observation_root.is_some(),
                );
            }
            PlanState::Unsupported => {}
        }

        &self.snapshot
    }

    pub fn snapshot(&self) -> &GovernanceSnapshot {
        &self.snapshot
    }

    pub fn cgroup_path(&self) -> Option<&Path> {
        self.cgroup_path.as_deref()
    }

    fn prepare_linux(environment: &GovernanceEnvironment, request: GovernanceRequest) -> Self {
        let Some(root) = environment.linux_cgroup_root.clone() else {
            return Self::unavailable(
                GovernancePlatform::Linux,
                request,
                "linux_cgroup",
                "cgroup v2 root is unavailable on this host".to_string(),
            );
        };

        let controllers = read_controllers(&root);
        let cgroup_path = root.join("execmanager").join(&request.exec_id);
        let mut gaps = Vec::new();

        if fs::create_dir_all(&cgroup_path).is_err() {
            return Self::unavailable(
                GovernancePlatform::Linux,
                request,
                "linux_cgroup",
                format!("unable to prepare cgroup path {}", cgroup_path.display()),
            );
        }

        if request.profile.memory_max_bytes.is_some()
            && !controllers.iter().any(|name| name == "memory")
        {
            push_gap(
                &mut gaps,
                "memory_controller",
                "memory controller is unavailable; memory enforcement is degraded".to_string(),
            );
        }

        if request.profile.cpu_max_micros.is_some() && !controllers.iter().any(|name| name == "cpu")
        {
            push_gap(
                &mut gaps,
                "cpu_controller",
                "cpu controller is unavailable; cpu enforcement is degraded".to_string(),
            );
        }

        ensure_file(&cgroup_path.join("cgroup.procs"));

        if let Some(memory_max) = request.profile.memory_max_bytes {
            if controllers.iter().any(|name| name == "memory") {
                if let Err(error) =
                    fs::write(cgroup_path.join("memory.max"), format!("{memory_max}\n"))
                {
                    push_gap(
                        &mut gaps,
                        "memory_controller",
                        format!("failed to write memory.max: {error}"),
                    );
                }
            }
        }

        if let Some(cpu_max) = request.profile.cpu_max_micros {
            if controllers.iter().any(|name| name == "cpu") {
                let cpu_period = request.profile.cpu_period_micros.unwrap_or(100_000);
                if let Err(error) = fs::write(
                    cgroup_path.join("cpu.max"),
                    format!("{cpu_max} {cpu_period}\n"),
                ) {
                    push_gap(
                        &mut gaps,
                        "cpu_controller",
                        format!("failed to write cpu.max: {error}"),
                    );
                }
            }
        }

        let enforcement_requested =
            request.profile.memory_max_bytes.is_some() || request.profile.cpu_max_micros.is_some();
        let capability = compute_capability(
            enforcement_requested,
            false,
            !gaps.is_empty(),
            has_observation_path(&cgroup_path),
        );

        Self {
            cgroup_path: Some(cgroup_path.clone()),
            state: PlanState::Linux(LinuxPlanState {
                cgroup_path: cgroup_path.clone(),
                enforcement_requested,
            }),
            snapshot: GovernanceSnapshot {
                platform: GovernancePlatform::Linux,
                capability,
                profile: request.profile,
                placement: PlacementState::Attempted {
                    reason: "cgroup prepared; pid placement not attempted yet".to_string(),
                },
                current: ResourceMetrics::default(),
                recent_peak: ResourceMetrics::default(),
                enforcement_triggered: false,
                enforcement_gaps: gaps,
            },
        }
    }

    fn prepare_macos(environment: &GovernanceEnvironment, request: GovernanceRequest) -> Self {
        let enforcement_requested =
            request.profile.memory_max_bytes.is_some() || request.profile.cpu_max_micros.is_some();
        let mut gaps = vec![EnforcementGap {
            scope: "macos_degraded".to_string(),
            reason: "degraded: macOS has no cgroup placement path; degraded governance must stay explicit"
                .to_string(),
        }];

        if request.profile.cpu_max_micros.is_some() {
            push_gap(
                &mut gaps,
                "cpu_controller",
                "degraded: macOS cannot provide Linux-style cpu quota enforcement".to_string(),
            );
        }

        if enforcement_requested && !environment.macos_setrlimit_best_effort {
            push_gap(
                &mut gaps,
                "setrlimit",
                "degraded: setrlimit-style enforcement is unavailable; falling back to observation"
                    .to_string(),
            );
        }

        let capability = compute_capability(
            enforcement_requested,
            environment.macos_setrlimit_best_effort,
            !gaps.is_empty(),
            environment.macos_observation_root.is_some(),
        );

        Self {
            cgroup_path: None,
            state: PlanState::MacOs(MacOsPlanState {
                observation_root: environment.macos_observation_root.clone(),
                setrlimit_best_effort: environment.macos_setrlimit_best_effort,
                enforcement_requested,
            }),
            snapshot: GovernanceSnapshot {
                platform: GovernancePlatform::MacOs,
                capability,
                profile: request.profile,
                placement: PlacementState::NotApplicable {
                    reason:
                        "macOS has no cgroup placement path; degraded governance must stay explicit"
                            .to_string(),
                },
                current: ResourceMetrics::default(),
                recent_peak: ResourceMetrics::default(),
                enforcement_triggered: false,
                enforcement_gaps: gaps,
            },
        }
    }

    fn unsupported(request: GovernanceRequest) -> Self {
        Self::unavailable(
            GovernancePlatform::Unsupported,
            request,
            "unsupported_platform",
            "resource governance is unavailable on this platform".to_string(),
        )
    }

    fn unavailable(
        platform: GovernancePlatform,
        request: GovernanceRequest,
        scope: &str,
        reason: String,
    ) -> Self {
        Self {
            cgroup_path: None,
            state: PlanState::Unsupported,
            snapshot: GovernanceSnapshot {
                platform,
                capability: GovernanceCapability::Unavailable,
                profile: request.profile,
                placement: PlacementState::NotApplicable {
                    reason: reason.clone(),
                },
                current: ResourceMetrics::default(),
                recent_peak: ResourceMetrics::default(),
                enforcement_triggered: false,
                enforcement_gaps: vec![EnforcementGap {
                    scope: scope.to_string(),
                    reason,
                }],
            },
        }
    }
}

#[derive(Debug, Clone)]
enum PlanState {
    Linux(LinuxPlanState),
    MacOs(MacOsPlanState),
    Unsupported,
}

#[derive(Debug, Clone)]
struct LinuxPlanState {
    cgroup_path: PathBuf,
    enforcement_requested: bool,
}

#[derive(Debug, Clone)]
struct MacOsPlanState {
    observation_root: Option<PathBuf>,
    setrlimit_best_effort: bool,
    enforcement_requested: bool,
}

#[derive(Debug, Clone)]
pub struct GovernanceEnvironment {
    platform: GovernancePlatform,
    linux_cgroup_root: Option<PathBuf>,
    macos_observation_root: Option<PathBuf>,
    macos_setrlimit_best_effort: bool,
}

impl GovernanceEnvironment {
    pub fn linux_for_tests(root: impl Into<PathBuf>) -> Self {
        Self {
            platform: GovernancePlatform::Linux,
            linux_cgroup_root: Some(root.into()),
            macos_observation_root: None,
            macos_setrlimit_best_effort: false,
        }
    }

    pub fn macos_for_tests(
        observation_root: impl Into<PathBuf>,
        setrlimit_best_effort: bool,
    ) -> Self {
        Self {
            platform: GovernancePlatform::MacOs,
            linux_cgroup_root: None,
            macos_observation_root: Some(observation_root.into()),
            macos_setrlimit_best_effort: setrlimit_best_effort,
        }
    }

    pub fn current() -> Self {
        #[cfg(target_os = "linux")]
        {
            Self::linux_for_tests("/sys/fs/cgroup")
        }

        #[cfg(target_os = "macos")]
        {
            Self {
                platform: GovernancePlatform::MacOs,
                linux_cgroup_root: None,
                macos_observation_root: None,
                macos_setrlimit_best_effort: true,
            }
        }

        #[cfg(not(any(target_os = "linux", target_os = "macos")))]
        {
            Self {
                platform: GovernancePlatform::Unsupported,
                linux_cgroup_root: None,
                macos_observation_root: None,
                macos_setrlimit_best_effort: false,
            }
        }
    }
}

fn push_gap(gaps: &mut Vec<EnforcementGap>, scope: &str, reason: String) {
    gaps.push(EnforcementGap {
        scope: scope.to_string(),
        reason,
    });
}

fn ensure_file(path: &Path) {
    if !path.exists() {
        let _ = fs::write(path, b"");
    }
}

fn read_controllers(root: &Path) -> Vec<String> {
    fs::read_to_string(root.join("cgroup.controllers"))
        .map(|controllers| {
            controllers
                .split_whitespace()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default()
}

fn read_linux_metrics(root: &Path) -> ResourceMetrics {
    let cpu_usage_micros = fs::read_to_string(root.join("cpu.stat"))
        .ok()
        .and_then(|content| {
            content.lines().find_map(|line| {
                let mut parts = line.split_whitespace();
                match (parts.next(), parts.next()) {
                    (Some("usage_usec"), Some(value)) => value.parse::<u64>().ok(),
                    _ => None,
                }
            })
        });

    ResourceMetrics {
        memory_current_bytes: read_u64(root.join("memory.current")),
        memory_peak_bytes: read_u64(root.join("memory.peak")),
        cpu_usage_micros,
    }
}

fn read_observation_metrics(root: Option<&Path>) -> ResourceMetrics {
    let Some(root) = root else {
        return ResourceMetrics::default();
    };

    ResourceMetrics {
        memory_current_bytes: read_u64(root.join("memory.current")),
        memory_peak_bytes: read_u64(root.join("memory.peak")),
        cpu_usage_micros: fs::read_to_string(root.join("cpu.stat"))
            .ok()
            .and_then(|content| {
                content.lines().find_map(|line| {
                    let mut parts = line.split_whitespace();
                    match (parts.next(), parts.next()) {
                        (Some("usage_usec"), Some(value)) => value.parse::<u64>().ok(),
                        _ => None,
                    }
                })
            }),
    }
}

fn read_u64(path: impl AsRef<Path>) -> Option<u64> {
    fs::read_to_string(path)
        .ok()
        .and_then(|value| value.trim().parse::<u64>().ok())
}

fn linux_enforcement_triggered(root: &Path) -> bool {
    fs::read_to_string(root.join("memory.events"))
        .ok()
        .map(|content| {
            content.lines().any(|line| {
                let mut parts = line.split_whitespace();
                matches!(
                    (parts.next(), parts.next()),
                    (Some("high" | "max" | "oom" | "oom_kill"), Some(value))
                        if value.parse::<u64>().ok().unwrap_or(0) > 0
                )
            })
        })
        .unwrap_or(false)
}

fn has_observation_path(root: &Path) -> bool {
    root.exists()
}

fn compute_capability(
    enforcement_requested: bool,
    enforcement_applied: bool,
    has_gaps: bool,
    has_observation: bool,
) -> GovernanceCapability {
    if enforcement_requested {
        if enforcement_applied && !has_gaps {
            GovernanceCapability::FullyEnforced
        } else if enforcement_applied || has_observation {
            GovernanceCapability::PartiallyEnforced
        } else {
            GovernanceCapability::Unavailable
        }
    } else if has_observation {
        GovernanceCapability::ObservableOnly
    } else {
        GovernanceCapability::Unavailable
    }
}

pub fn capture_current_platform_governance(
    exec_id: impl Into<String>,
    profile: ResourceProfile,
    pid: u32,
) -> GovernanceSnapshot {
    let coordinator = GovernanceCoordinator::for_environment(GovernanceEnvironment::current());
    let mut plan = coordinator.prepare(GovernanceRequest::new(exec_id, profile));
    plan.apply_to_pid(pid);
    plan.capture().clone()
}

pub fn apply_governance_to_current_process(
    cgroup_path: impl AsRef<Path>,
    pid: u32,
) -> io::Result<()> {
    fs::write(
        cgroup_path.as_ref().join("cgroup.procs"),
        format!("{pid}\n"),
    )
}

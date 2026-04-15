use std::fs;

use execmanager_platform::{
    GovernanceCapability, GovernanceCoordinator, GovernanceEnvironment, GovernancePlatform,
    GovernanceRequest, PlacementState, ResourceProfile,
};
use tempfile::tempdir;

#[test]
fn linux_cgroup_enforcement_path_works() {
    let temp = tempdir().expect("tempdir");
    let root = temp.path().join("cgroup2");
    fs::create_dir_all(&root).expect("create cgroup root");
    fs::write(root.join("cgroup.controllers"), "cpu memory").expect("write controllers");

    let request = GovernanceRequest::new(
        "exec-linux-cgroup-001",
        ResourceProfile {
            memory_max_bytes: Some(64 * 1024 * 1024),
            cpu_max_micros: Some(50_000),
            cpu_period_micros: Some(100_000),
        },
    );
    let coordinator =
        GovernanceCoordinator::for_environment(GovernanceEnvironment::linux_for_tests(&root));
    let mut plan = coordinator.prepare(request.clone());

    let cgroup_path = plan
        .cgroup_path()
        .expect("linux plan should expose a cgroup placement path")
        .to_path_buf();

    fs::write(cgroup_path.join("memory.current"), "4096\n").expect("seed memory current");
    fs::write(cgroup_path.join("memory.peak"), "8192\n").expect("seed memory peak");
    fs::write(cgroup_path.join("cpu.stat"), "usage_usec 1234\n").expect("seed cpu stat");

    plan.apply_to_pid(4242);
    let snapshot = plan.capture().clone();

    assert_eq!(snapshot.platform, GovernancePlatform::Linux);
    assert_eq!(snapshot.capability, GovernanceCapability::FullyEnforced);
    assert_eq!(snapshot.profile, request.profile);
    assert_eq!(
        snapshot.placement,
        PlacementState::Applied {
            target: cgroup_path.display().to_string(),
        }
    );
    assert_eq!(snapshot.current.memory_current_bytes, Some(4096));
    assert_eq!(snapshot.recent_peak.memory_peak_bytes, Some(8192));
    assert_eq!(snapshot.current.cpu_usage_micros, Some(1234));
    assert!(!snapshot.enforcement_triggered);
    assert!(snapshot.enforcement_gaps.is_empty());

    let cgroup_procs = fs::read_to_string(cgroup_path.join("cgroup.procs")).expect("read procs");
    assert_eq!(cgroup_procs.trim(), "4242");
    assert_eq!(
        fs::read_to_string(cgroup_path.join("memory.max")).expect("read memory max"),
        format!("{}\n", 64 * 1024 * 1024)
    );
    assert_eq!(
        fs::read_to_string(cgroup_path.join("cpu.max")).expect("read cpu max"),
        "50000 100000\n"
    );
}

#[test]
fn degraded_capability_is_explicit() {
    let temp = tempdir().expect("tempdir");
    let observation_root = temp.path().join("macos-observation");
    fs::create_dir_all(&observation_root).expect("create observation root");
    fs::write(observation_root.join("memory.current"), "1024\n").expect("seed memory current");
    fs::write(observation_root.join("memory.peak"), "2048\n").expect("seed memory peak");

    let mut plan = GovernanceCoordinator::for_environment(GovernanceEnvironment::macos_for_tests(
        &observation_root,
        true,
    ))
    .prepare(GovernanceRequest::new(
        "exec-macos-001",
        ResourceProfile {
            memory_max_bytes: Some(32 * 1024 * 1024),
            cpu_max_micros: Some(25_000),
            cpu_period_micros: Some(100_000),
        },
    ));

    plan.apply_to_pid(777);
    let snapshot = plan.capture().clone();

    assert_eq!(snapshot.platform, GovernancePlatform::MacOs);
    assert_eq!(snapshot.capability, GovernanceCapability::PartiallyEnforced);
    assert_eq!(
        snapshot.placement,
        PlacementState::NotApplicable {
            reason: "macOS has no cgroup placement path; degraded governance must stay explicit"
                .to_string(),
        }
    );
    assert_eq!(snapshot.current.memory_current_bytes, Some(1024));
    assert_eq!(snapshot.recent_peak.memory_peak_bytes, Some(2048));
    assert!(!snapshot.enforcement_gaps.is_empty());
    assert!(snapshot
        .enforcement_gaps
        .iter()
        .any(|gap| gap.reason.contains("degraded")));
}

#[test]
fn observable_only_capability_is_explicit_when_setrlimit_is_unavailable() {
    let temp = tempdir().expect("tempdir");
    let observation_root = temp.path().join("macos-observation-only");
    fs::create_dir_all(&observation_root).expect("create observation root");
    fs::write(observation_root.join("memory.current"), "4096\n").expect("seed memory current");
    fs::write(observation_root.join("memory.peak"), "8192\n").expect("seed memory peak");

    let mut plan = GovernanceCoordinator::for_environment(GovernanceEnvironment::macos_for_tests(
        &observation_root,
        false,
    ))
    .prepare(GovernanceRequest::new(
        "exec-macos-observable-001",
        ResourceProfile::default(),
    ));

    plan.apply_to_pid(888);
    let snapshot = plan.capture().clone();

    assert_eq!(snapshot.platform, GovernancePlatform::MacOs);
    assert_eq!(snapshot.capability, GovernanceCapability::ObservableOnly);
    assert_eq!(snapshot.current.memory_current_bytes, Some(4096));
    assert_eq!(snapshot.recent_peak.memory_peak_bytes, Some(8192));
    assert_eq!(
        snapshot.placement,
        PlacementState::NotApplicable {
            reason: "macOS has no cgroup placement path; degraded governance must stay explicit"
                .to_string(),
        }
    );
    assert!(!snapshot.enforcement_gaps.is_empty());
    assert!(snapshot
        .enforcement_gaps
        .iter()
        .any(|gap| gap.reason.contains("setrlimit") || gap.reason.contains("degraded")));
}

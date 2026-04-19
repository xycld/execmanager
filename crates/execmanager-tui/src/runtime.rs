use std::{cmp::Reverse, fs, path::Path, time::Duration};

use execmanager_contracts::ProjectionState;
use execmanager_daemon::{
    ExecutionView, JournalEvent, LifecycleStage, RecordedJournalEvent, RuntimeProjection,
};

use crate::{app::ViewMode, RenderError};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PressureLevel {
    Healthy,
    Elevated,
    Constrained,
    Unknown,
}

impl PressureLevel {
    pub fn label(self) -> &'static str {
        match self {
            Self::Healthy => "healthy",
            Self::Elevated => "elevated",
            Self::Constrained => "constrained",
            Self::Unknown => "unknown",
        }
    }

    pub fn ansi_color(self) -> &'static str {
        match self {
            Self::Healthy => "\x1b[32m",
            Self::Elevated => "\x1b[33m",
            Self::Constrained => "\x1b[31m",
            Self::Unknown => "\x1b[90m",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DashboardView {
    key: String,
    pub title: String,
    pub subtitle: String,
    pub pressure: PressureLevel,
    pub detail_lines: Vec<String>,
}

impl DashboardView {
    pub fn new(
        key: impl Into<String>,
        title: impl Into<String>,
        subtitle: impl Into<String>,
        pressure: PressureLevel,
        detail_lines: Vec<String>,
    ) -> Self {
        Self {
            key: key.into(),
            title: title.into(),
            subtitle: subtitle.into(),
            pressure,
            detail_lines,
        }
    }

    pub fn key(&self) -> &str {
        &self.key
    }
}

#[derive(Debug, Clone, Default)]
pub struct DashboardModel {
    pub instances: Vec<DashboardView>,
    pub services: Vec<DashboardView>,
    pub history: Vec<DashboardView>,
    pub ghosts: Vec<DashboardView>,
}

impl DashboardModel {
    pub fn view(&self, mode: ViewMode) -> &[DashboardView] {
        match mode {
            ViewMode::Instances => &self.instances,
            ViewMode::Services => &self.services,
            ViewMode::History => &self.history,
            ViewMode::Ghosts => &self.ghosts,
        }
    }
}

pub fn load_dashboard_model(journal_path: &Path) -> Result<DashboardModel, RenderError> {
    let projection =
        RuntimeProjection::replay_from_path(journal_path).map_err(RenderError::History)?;
    build_dashboard_model(&projection)
}

pub fn build_dashboard_model(
    projection: &RuntimeProjection,
) -> Result<DashboardModel, RenderError> {
    let history = match projection.history() {
        Ok(history) => history,
        Err(_) if projection.is_degraded() => Vec::new(),
        Err(error) => return Err(RenderError::History(error)),
    };

    Ok(DashboardModel {
        instances: build_instance_views(projection, &history),
        services: build_service_views(projection, &history),
        history: build_history_views(&history),
        ghosts: build_ghost_views(projection, &history),
    })
}

pub fn detail_for_exec(
    projection: &RuntimeProjection,
    exec_id: &str,
) -> Result<Option<Vec<String>>, RenderError> {
    let history = match projection.history() {
        Ok(history) => history,
        Err(_) if projection.is_degraded() => Vec::new(),
        Err(error) => return Err(RenderError::History(error)),
    };

    Ok(projection
        .execution(exec_id)
        .map(|execution| build_instance_detail(projection, execution, &history)))
}

fn build_instance_views(
    projection: &RuntimeProjection,
    history: &[RecordedJournalEvent],
) -> Vec<DashboardView> {
    let mut launch_order = Vec::new();
    for record in history {
        if let JournalEvent::LaunchRequested { exec_id, .. } = &record.event {
            if !launch_order
                .iter()
                .any(|(existing, _)| existing == exec_id.as_str())
            {
                launch_order.push((exec_id.as_str().to_string(), record.offset));
            }
        }
    }

    launch_order.sort_by_key(|(_, offset)| Reverse(*offset));

    launch_order
        .into_iter()
        .filter_map(|(exec_id, _)| {
            let execution = projection.execution(&exec_id)?;
            is_running(execution).then(|| build_instance_view(projection, execution, history))
        })
        .collect()
}

fn build_instance_view(
    projection: &RuntimeProjection,
    execution: &ExecutionView,
    history: &[RecordedJournalEvent],
) -> DashboardView {
    let runtime = runtime_label(execution);
    let source = source_label(projection, execution);
    let cwd = cwd_label(projection, execution);
    let pressure = pressure_level(execution);

    DashboardView {
        key: execution.exec_id.as_str().to_string(),
        title: summarize_command(&execution.original_command),
        subtitle: format!("{} | {} | {} | {}", runtime, source, cwd, pressure.label()),
        pressure,
        detail_lines: build_instance_detail(projection, execution, history),
    }
}

fn build_instance_detail(
    projection: &RuntimeProjection,
    execution: &ExecutionView,
    history: &[RecordedJournalEvent],
) -> Vec<String> {
    let mut lines = vec![
        "Identity".to_string(),
        format!("exec id: {}", execution.exec_id.as_str()),
        format!(
            "summary: {}",
            summarize_command(&execution.original_command)
        ),
        format!("original command: {}", execution.original_command),
        format!(
            "rewritten launch spec: {}",
            execution
                .rewritten_command
                .as_deref()
                .unwrap_or(&execution.command)
        ),
        format!("source: {}", source_label(projection, execution)),
        format!("cwd: {}", cwd_label(projection, execution)),
        String::new(),
        "Runtime".to_string(),
        format!(
            "effective state: {}",
            projection_state_label(&execution.state)
        ),
        format!(
            "observed state: {}",
            projection_state_label(&execution.observed_state)
        ),
        format!(
            "runtime state: {} (observed {})",
            projection_state_label(&execution.state),
            projection_state_label(&execution.observed_state)
        ),
        format!("runtime: {}", runtime_label(execution)),
        format!(
            "policy: {}",
            execution
                .policy_outcome
                .as_ref()
                .map(policy_summary)
                .unwrap_or_else(|| "none recorded".to_string())
        ),
        String::new(),
        "Resources".to_string(),
        format!("pressure: {}", pressure_level(execution).label()),
        format!("current memory: {}", memory_current_label(execution)),
        format!("cpu usage: {}", cpu_usage_label(execution)),
        format!("recent peak memory: {}", memory_peak_label(execution)),
        format!("resource governance: {}", governance_label(execution)),
        String::new(),
        "Service / Ports".to_string(),
    ];

    if let Some(governance) = &execution.resource_governance {
        if is_observability_degraded(governance) || projection.is_degraded() {
            lines.push("observability degraded".to_string());
        }
        for gap in &governance.enforcement_gaps {
            lines.push(gap.reason.clone());
        }
    }

    let services = collect_services_for_execution(projection, history, execution.exec_id.as_str());
    if services.is_empty() {
        lines.push("no observed services".to_string());
    } else {
        lines.extend(services);
    }

    if let Some(ghost) = projection.ghost(execution.exec_id.as_str()) {
        lines.push(format!(
            "ghost {}: {}",
            projection_state_label(&ghost.state),
            ghost.detail
        ));
    }

    if let Some(reason) = projection.degraded_reason() {
        lines.push(format!("replay degraded: {reason}"));
    }

    lines.push(String::new());
    lines.push("Logs".to_string());
    lines.extend(log_tail_lines(execution));
    lines.push(String::new());
    lines.push("Recent events".to_string());
    let mut any_event = false;
    for record in history {
        if event_exec_id(&record.event) == Some(execution.exec_id.as_str()) {
            any_event = true;
            lines.push(event_name(&record.event).to_string());
        }
    }
    if !any_event {
        lines.push("no recent events".to_string());
    }
    lines
}

fn build_service_views(
    projection: &RuntimeProjection,
    history: &[RecordedJournalEvent],
) -> Vec<DashboardView> {
    let mut rows = Vec::new();
    for record in history {
        if let JournalEvent::ServiceObserved { exec_id, service } = &record.event {
            if let Some(current) = projection.service(exec_id.as_str(), &service.name) {
                let key = format!("{}:{}", exec_id.as_str(), current.name);
                if rows.iter().any(|row: &DashboardView| row.key() == key) {
                    continue;
                }
                rows.push(DashboardView {
                    key,
                    title: current.name.clone(),
                    subtitle: format!(
                        "{} | {}",
                        exec_id.as_str(),
                        if current.port_ids.is_empty() {
                            "no ports".to_string()
                        } else {
                            current.port_ids.join(", ")
                        }
                    ),
                    pressure: PressureLevel::Unknown,
                    detail_lines: vec![
                        "Service / Ports".to_string(),
                        format!("exec id: {}", exec_id.as_str()),
                        format!("service: {}", current.name),
                        format!("state: {}", projection_state_label(&current.state)),
                        format!(
                            "ports: {}",
                            if current.port_ids.is_empty() {
                                "none".to_string()
                            } else {
                                current.port_ids.join(", ")
                            }
                        ),
                    ],
                });
            }
        }
    }
    rows
}

fn build_history_views(history: &[RecordedJournalEvent]) -> Vec<DashboardView> {
    history
        .iter()
        .rev()
        .map(|record| DashboardView {
            key: format!("history:{}", record.offset),
            title: event_name(&record.event).to_string(),
            subtitle: format!("offset {}", record.offset),
            pressure: PressureLevel::Unknown,
            detail_lines: vec![
                "History".to_string(),
                format!("offset: {}", record.offset),
                format!("event: {}", event_name(&record.event)),
            ],
        })
        .collect()
}

fn build_ghost_views(
    projection: &RuntimeProjection,
    history: &[RecordedJournalEvent],
) -> Vec<DashboardView> {
    let mut rows = Vec::new();
    let mut seen = Vec::new();
    for record in history {
        if let Some(exec_id) = event_exec_id(&record.event) {
            if seen.iter().any(|existing: &String| existing == exec_id) {
                continue;
            }
            if let Some(ghost) = projection.ghost(exec_id) {
                seen.push(exec_id.to_string());
                rows.push(DashboardView {
                    key: exec_id.to_string(),
                    title: exec_id.to_string(),
                    subtitle: format!(
                        "{} | {}",
                        projection_state_label(&ghost.state),
                        ghost.detail
                    ),
                    pressure: PressureLevel::Unknown,
                    detail_lines: vec![
                        "Ghosts / Reconcile".to_string(),
                        format!("exec id: {}", exec_id),
                        format!("state: {}", projection_state_label(&ghost.state)),
                        format!("detail: {}", ghost.detail),
                    ],
                });
            }
        }
    }
    rows
}

fn collect_services_for_execution(
    projection: &RuntimeProjection,
    history: &[RecordedJournalEvent],
    exec_id: &str,
) -> Vec<String> {
    let mut lines = Vec::new();
    let mut seen = Vec::new();
    for record in history {
        if let JournalEvent::ServiceObserved {
            exec_id: observed_exec_id,
            service,
        } = &record.event
        {
            if observed_exec_id.as_str() != exec_id
                || seen.iter().any(|name: &String| name == &service.name)
            {
                continue;
            }
            seen.push(service.name.clone());
            if let Some(current) = projection.service(exec_id, &service.name) {
                lines.push(format!(
                    "service {} -> {} [{}]",
                    current.name,
                    if current.port_ids.is_empty() {
                        "no ports".to_string()
                    } else {
                        current.port_ids.join(", ")
                    },
                    projection_state_label(&current.state)
                ));
            }
        }
    }
    lines
}

fn summarize_command(command: &str) -> String {
    const LIMIT: usize = 48;
    if command.chars().count() <= LIMIT {
        return command.to_string();
    }
    let mut summary: String = command.chars().take(LIMIT - 1).collect();
    summary.push('…');
    summary
}

fn source_label(projection: &RuntimeProjection, execution: &ExecutionView) -> String {
    projection
        .env_snapshot(execution.exec_id.as_str())
        .and_then(|snapshot| {
            snapshot
                .entries
                .iter()
                .find(|entry| entry.name == "EXECMANAGER_SOURCE")
                .and_then(|entry| entry.value.clone())
        })
        .unwrap_or_else(|| "unknown".to_string())
}

fn cwd_label(projection: &RuntimeProjection, execution: &ExecutionView) -> String {
    projection
        .env_snapshot(execution.exec_id.as_str())
        .and_then(|snapshot| {
            snapshot
                .entries
                .iter()
                .find(|entry| entry.name == "PWD")
                .and_then(|entry| entry.value.clone())
        })
        .map(|cwd| abbreviate_path(&cwd))
        .unwrap_or_else(|| "unknown".to_string())
}

fn abbreviate_path(path: &str) -> String {
    const LIMIT: usize = 32;
    if path.chars().count() <= LIMIT {
        return path.to_string();
    }
    let suffix: String = path
        .chars()
        .rev()
        .take(LIMIT - 1)
        .collect::<String>()
        .chars()
        .rev()
        .collect();
    format!("…{suffix}")
}

fn runtime_label(execution: &ExecutionView) -> String {
    execution
        .ownership
        .as_ref()
        .and_then(|ownership| ownership.start_time_ticks)
        .and_then(runtime_from_start_ticks)
        .unwrap_or_else(|| "unknown".to_string())
}

fn runtime_from_start_ticks(start_ticks: u64) -> Option<String> {
    #[cfg(target_os = "linux")]
    {
        let ticks_per_second = unsafe { libc::sysconf(libc::_SC_CLK_TCK) };
        if ticks_per_second <= 0 {
            return None;
        }
        let uptime = fs::read_to_string("/proc/uptime").ok()?;
        let uptime_secs = uptime.split_whitespace().next()?.parse::<f64>().ok()?;
        let start_secs = start_ticks as f64 / ticks_per_second as f64;
        let runtime = (uptime_secs - start_secs).max(0.0);
        Some(format_duration(Duration::from_secs_f64(runtime)))
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = start_ticks;
        None
    }
}

fn format_duration(duration: Duration) -> String {
    let secs = duration.as_secs();
    let hours = secs / 3600;
    let minutes = (secs % 3600) / 60;
    let seconds = secs % 60;
    if hours > 0 {
        format!("{hours}h {minutes}m")
    } else if minutes > 0 {
        format!("{minutes}m {seconds}s")
    } else {
        format!("{seconds}s")
    }
}

fn pressure_level(execution: &ExecutionView) -> PressureLevel {
    let Some(governance) = &execution.resource_governance else {
        return PressureLevel::Unknown;
    };
    if governance.enforcement_triggered {
        return PressureLevel::Constrained;
    }
    let memory = governance.current.memory_current_bytes.unwrap_or_default();
    let cpu = governance.current.cpu_usage_micros.unwrap_or_default();
    if memory >= 512 * 1024 * 1024 || cpu >= 200_000 {
        PressureLevel::Constrained
    } else if memory >= 128 * 1024 * 1024 || cpu >= 50_000 {
        PressureLevel::Elevated
    } else {
        PressureLevel::Healthy
    }
}

fn memory_current_label(execution: &ExecutionView) -> String {
    execution
        .resource_governance
        .as_ref()
        .and_then(|governance| governance.current.memory_current_bytes)
        .map(|bytes| bytes.to_string())
        .unwrap_or_else(|| "n/a".to_string())
}

fn memory_peak_label(execution: &ExecutionView) -> String {
    execution
        .resource_governance
        .as_ref()
        .and_then(|governance| governance.recent_peak.memory_peak_bytes)
        .map(|bytes| bytes.to_string())
        .unwrap_or_else(|| "n/a".to_string())
}

fn cpu_usage_label(execution: &ExecutionView) -> String {
    execution
        .resource_governance
        .as_ref()
        .and_then(|governance| governance.current.cpu_usage_micros)
        .map(|micros| format!("{micros}µs"))
        .unwrap_or_else(|| "n/a".to_string())
}

fn governance_label(execution: &ExecutionView) -> String {
    execution
        .resource_governance
        .as_ref()
        .map(|governance| {
            if governance.enforcement_triggered {
                "constrained".to_string()
            } else {
                match governance.capability {
                    execmanager_platform::GovernanceCapability::FullyEnforced => {
                        "fully_enforced".to_string()
                    }
                    execmanager_platform::GovernanceCapability::PartiallyEnforced => {
                        "partially_enforced".to_string()
                    }
                    execmanager_platform::GovernanceCapability::ObservableOnly => {
                        "observable_only".to_string()
                    }
                    execmanager_platform::GovernanceCapability::Unavailable => {
                        "unavailable".to_string()
                    }
                }
            }
        })
        .unwrap_or_else(|| "unknown".to_string())
}

fn policy_summary(outcome: &execmanager_daemon::LaunchPolicyOutcome) -> String {
    match outcome {
        execmanager_daemon::LaunchPolicyOutcome::AllowedAsRequested { policy, .. } => {
            format!("allowed_as_requested by {policy}")
        }
        execmanager_daemon::LaunchPolicyOutcome::Rewritten { policy, .. } => {
            format!("rewritten by {policy}")
        }
        execmanager_daemon::LaunchPolicyOutcome::Denied { policy, .. } => {
            format!("denied by {policy}")
        }
    }
}

fn is_observability_degraded(governance: &execmanager_platform::GovernanceSnapshot) -> bool {
    !matches!(
        governance.capability,
        execmanager_platform::GovernanceCapability::FullyEnforced
    ) || !governance.enforcement_gaps.is_empty()
}

fn log_tail_lines(execution: &ExecutionView) -> Vec<String> {
    let mut lines = Vec::new();
    if let Some(stdout) = &execution.stdout {
        lines.push("stdout:".to_string());
        lines.extend(read_blob_tail(&stdout.storage_path));
    }
    if let Some(stderr) = &execution.stderr {
        if !lines.is_empty() {
            lines.push(String::new());
        }
        lines.push("stderr:".to_string());
        lines.extend(read_blob_tail(&stderr.storage_path));
    }
    if lines.is_empty() {
        lines.push("no retained stdout/stderr".to_string());
    }
    lines
}

fn read_blob_tail(storage_path: &str) -> Vec<String> {
    match fs::read(storage_path) {
        Ok(bytes) => {
            let text = String::from_utf8_lossy(&bytes);
            let tail: Vec<String> = text.lines().rev().take(8).map(str::to_string).collect();
            let mut tail = tail.into_iter().rev().collect::<Vec<_>>();
            if tail.is_empty() {
                tail.push("<empty>".to_string());
            }
            tail
        }
        Err(error) => vec![format!("unable to read log tail: {error}")],
    }
}

fn is_running(execution: &ExecutionView) -> bool {
    execution.state != ProjectionState::Exited
        && execution.observed_state != ProjectionState::Exited
        && !execution.lifecycle.contains(&LifecycleStage::Exited)
}

fn projection_state_label(state: &ProjectionState) -> &'static str {
    match state {
        ProjectionState::Managed => "managed",
        ProjectionState::Service => "service",
        ProjectionState::ShortTask => "short_task",
        ProjectionState::Missing => "missing",
        ProjectionState::Orphaned => "orphaned",
        ProjectionState::Escaped => "escaped",
        ProjectionState::Detached => "detached",
        ProjectionState::Exited => "exited",
        ProjectionState::Unknown => "unknown",
    }
}

fn event_exec_id(event: &JournalEvent) -> Option<&str> {
    match event {
        JournalEvent::LaunchRequested { exec_id, .. }
        | JournalEvent::LaunchPolicyEvaluated { exec_id, .. }
        | JournalEvent::LaunchAdmitted { exec_id, .. }
        | JournalEvent::ProcessSpawned { exec_id, .. }
        | JournalEvent::ExecutionRegistered { exec_id, .. }
        | JournalEvent::ExecutionStateUpdated { exec_id, .. }
        | JournalEvent::ServiceObserved { exec_id, .. }
        | JournalEvent::ServiceOverrideApplied { exec_id, .. }
        | JournalEvent::PortObserved { exec_id, .. }
        | JournalEvent::GhostStateRecorded { exec_id, .. }
        | JournalEvent::ResourceGovernanceRecorded { exec_id, .. }
        | JournalEvent::HistorySnapshotRecorded { exec_id, .. } => Some(exec_id.as_str()),
    }
}

fn event_name(event: &JournalEvent) -> &'static str {
    match event {
        JournalEvent::LaunchRequested { .. } => "launch_requested",
        JournalEvent::LaunchPolicyEvaluated { .. } => "launch_policy_evaluated",
        JournalEvent::LaunchAdmitted { .. } => "launch_admitted",
        JournalEvent::ProcessSpawned { .. } => "process_spawned",
        JournalEvent::ExecutionRegistered { .. } => "execution_registered",
        JournalEvent::ExecutionStateUpdated { .. } => "execution_state_updated",
        JournalEvent::ServiceObserved { .. } => "service_observed",
        JournalEvent::ServiceOverrideApplied { .. } => "service_override_applied",
        JournalEvent::PortObserved { .. } => "port_observed",
        JournalEvent::GhostStateRecorded { .. } => "ghost_state_recorded",
        JournalEvent::ResourceGovernanceRecorded { .. } => "resource_governance_recorded",
        JournalEvent::HistorySnapshotRecorded { .. } => "history_snapshot_recorded",
    }
}

use std::collections::HashSet;

use execmanager_contracts::{ExecutionId, ProjectionState};
use execmanager_daemon::{
    ExecutionMode, ExecutionView, JournalEvent, LaunchPolicyOutcome, RecordedJournalEvent,
    ReplayError, RuntimeProjection,
};
use execmanager_platform::{GovernanceCapability, GovernanceSnapshot};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PrimaryView {
    Instances,
    Services,
    History,
    GhostsReconcile,
    InstanceDetail,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaneUiState {
    pub selected_exec_id: Option<ExecutionId>,
    pub focused_view: PrimaryView,
    pub instance_scroll: usize,
    pub service_scroll: usize,
    pub history_scroll: usize,
    pub ghost_scroll: usize,
}

#[derive(Debug)]
pub enum RenderError {
    History(ReplayError),
}

impl std::fmt::Display for RenderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::History(error) => write!(f, "unable to render history pane: {error}"),
        }
    }
}

impl std::error::Error for RenderError {}

impl Default for PaneUiState {
    fn default() -> Self {
        Self {
            selected_exec_id: None,
            focused_view: PrimaryView::Instances,
            instance_scroll: 0,
            service_scroll: 0,
            history_scroll: 0,
            ghost_scroll: 0,
        }
    }
}

pub fn render_screen(
    projection: &RuntimeProjection,
    ui: &PaneUiState,
) -> Result<String, RenderError> {
    let history = match projection.history() {
        Ok(history) => history,
        Err(_) if projection.is_degraded() => Vec::new(),
        Err(error) => return Err(RenderError::History(error)),
    };
    let exec_ids = execution_ids_from_history(&history, projection, ui.selected_exec_id.as_ref());
    let selected_exec_id = ui
        .selected_exec_id
        .as_ref()
        .map(|exec_id| exec_id.as_str().to_string())
        .or_else(|| exec_ids.first().cloned());
    let selected = selected_exec_id
        .as_deref()
        .and_then(|exec_id| projection.execution(exec_id));

    let sections = [
        render_instances(projection, &exec_ids, selected_exec_id.as_deref()),
        render_services(projection, &history, selected_exec_id.as_deref()),
        render_history(&history, selected_exec_id.as_deref()),
        render_ghosts(projection, &exec_ids),
        render_detail(projection, selected, &history),
    ];

    Ok(sections.join("\n\n"))
}

fn execution_ids_from_history(
    history: &[RecordedJournalEvent],
    projection: &RuntimeProjection,
    selected_exec_id: Option<&ExecutionId>,
) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut exec_ids = Vec::new();

    if let Some(selected_exec_id) = selected_exec_id {
        if projection.execution(selected_exec_id.as_str()).is_some() {
            seen.insert(selected_exec_id.as_str().to_string());
            exec_ids.push(selected_exec_id.as_str().to_string());
        }
    }

    for record in history {
        if let Some(exec_id) = event_exec_id(&record.event) {
            let key = exec_id.to_string();
            if seen.insert(key.clone()) {
                exec_ids.push(key);
            }
        }
    }

    exec_ids
}

fn render_instances(
    projection: &RuntimeProjection,
    exec_ids: &[String],
    selected_exec_id: Option<&str>,
) -> String {
    let mut lines = vec!["Instances".to_string()];
    if exec_ids.is_empty() {
        lines.push("- no executions".to_string());
    }

    for exec_id in exec_ids {
        let marker = if Some(exec_id.as_str()) == selected_exec_id {
            '*'
        } else {
            '-'
        };
        if let Some(execution) = projection.execution(exec_id) {
            let mut summary = format!(
                "{marker} {} | effective state: {} | observed: {}",
                execution.exec_id.as_str(),
                state_name(&execution.state),
                state_name(&execution.observed_state)
            );
            if projection.is_degraded() {
                summary.push_str(" | replay degraded");
            }
            if let Some(governance) = &execution.resource_governance {
                if is_observability_degraded(governance) {
                    summary.push_str(" | observability degraded");
                }
            }
            lines.push(summary);
        }
    }

    lines.join("\n")
}

fn render_services(
    projection: &RuntimeProjection,
    history: &[RecordedJournalEvent],
    selected_exec_id: Option<&str>,
) -> String {
    let mut lines = vec!["Services".to_string()];
    let Some(selected_exec_id) = selected_exec_id else {
        lines.push("- no selected execution".to_string());
        return lines.join("\n");
    };

    let mut seen = HashSet::new();
    for record in history {
        if let JournalEvent::ServiceObserved { exec_id, service } = &record.event {
            if exec_id.as_str() != selected_exec_id || !seen.insert(service.name.clone()) {
                continue;
            }

            if let Some(current_service) = projection.service(exec_id.as_str(), &service.name) {
                let port_list = if current_service.port_ids.is_empty() {
                    "no observed ports".to_string()
                } else {
                    current_service.port_ids.join(", ")
                };
                lines.push(format!(
                    "- service {} -> {} [{}]",
                    current_service.name,
                    port_list,
                    state_name(&current_service.state)
                ));
            }
        }
    }

    if lines.len() == 1 {
        lines.push("- no observed services".to_string());
    }

    lines.join("\n")
}

fn render_history(history: &[RecordedJournalEvent], selected_exec_id: Option<&str>) -> String {
    let mut lines = vec!["History".to_string()];
    for record in history {
        let matches_selection = selected_exec_id
            .map(|selected| event_exec_id(&record.event) == Some(selected))
            .unwrap_or(true);
        if !matches_selection {
            continue;
        }

        lines.push(format!(
            "- {} @{}",
            event_name(&record.event),
            record.offset
        ));
    }

    if lines.len() == 1 {
        lines.push("- no replay history".to_string());
    }

    lines.join("\n")
}

fn render_ghosts(projection: &RuntimeProjection, exec_ids: &[String]) -> String {
    let mut lines = vec!["Ghosts/Reconcile".to_string()];
    for exec_id in exec_ids {
        if let Some(ghost) = projection.ghost(exec_id) {
            lines.push(format!(
                "- ghost {}: {}",
                state_name(&ghost.state),
                ghost.detail
            ));
        }
    }

    if lines.len() == 1 {
        lines.push("- no ghost or reconcile warnings".to_string());
    }

    lines.join("\n")
}

fn render_detail(
    projection: &RuntimeProjection,
    selected: Option<&ExecutionView>,
    history: &[RecordedJournalEvent],
) -> String {
    let mut lines = vec!["Instance Detail".to_string()];
    let Some(execution) = selected else {
        lines.push("- no selected execution".to_string());
        return lines.join("\n");
    };

    lines.push(format!("exec_id: {}", execution.exec_id.as_str()));
    lines.push(format!("effective state: {}", state_name(&execution.state)));
    lines.push(format!(
        "runtime state: {} (observed {})",
        state_name(&execution.state),
        state_name(&execution.observed_state)
    ));
    lines.push(format!("original command: {}", execution.original_command));
    lines.push(format!(
        "rewritten launch spec: {}",
        execution
            .rewritten_command
            .as_deref()
            .unwrap_or(&execution.command)
    ));
    lines.push(format!(
        "policy: {}",
        execution
            .policy_outcome
            .as_ref()
            .map(policy_summary)
            .unwrap_or_else(|| "none recorded".to_string())
    ));
    if let Some(mode) = &execution.mode {
        lines.push(format!("mode: {}", execution_mode_name(mode)));
    }
    if let Some(ownership) = &execution.ownership {
        lines.push(format!(
            "ownership: pid={} pgid={} session={:?} start_ticks={:?}",
            ownership.root_pid,
            ownership.process_group_id,
            ownership.session_id,
            ownership.start_time_ticks
        ));
    }

    if let Some(governance) = &execution.resource_governance {
        lines.push(format!(
            "resource governance: {}",
            governance_capability_name(&governance.capability)
        ));
        lines.push(format!(
            "current memory: {}",
            governance
                .current
                .memory_current_bytes
                .map(|value| value.to_string())
                .unwrap_or_else(|| "n/a".to_string())
        ));
        lines.push(format!(
            "recent peak memory: {}",
            governance
                .recent_peak
                .memory_peak_bytes
                .map(|value| value.to_string())
                .unwrap_or_else(|| "n/a".to_string())
        ));
        if is_observability_degraded(governance) || projection.is_degraded() {
            lines.push("observability degraded".to_string());
        }
        for gap in &governance.enforcement_gaps {
            lines.push(gap.reason.clone());
        }
    }

    if let Some(reason) = projection.degraded_reason() {
        lines.push(format!("replay degraded: {reason}"));
    }

    if let Some(ghost) = projection.ghost(execution.exec_id.as_str()) {
        lines.push(format!(
            "ghost {}: {}",
            state_name(&ghost.state),
            ghost.detail
        ));
    }

    lines.push("audit timeline:".to_string());
    for record in history {
        if event_exec_id(&record.event) == Some(execution.exec_id.as_str()) {
            lines.push(format!("- {}", event_name(&record.event)));
        }
    }

    lines.join("\n")
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

fn state_name(state: &ProjectionState) -> &'static str {
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

fn policy_summary(outcome: &LaunchPolicyOutcome) -> String {
    match outcome {
        LaunchPolicyOutcome::AllowedAsRequested { policy, .. } => {
            format!("allowed_as_requested by {policy}")
        }
        LaunchPolicyOutcome::Rewritten { policy, .. } => format!("rewritten by {policy}"),
        LaunchPolicyOutcome::Denied { policy, .. } => format!("denied by {policy}"),
    }
}

fn execution_mode_name(mode: &ExecutionMode) -> &'static str {
    match mode {
        ExecutionMode::BatchPipes => "batch_pipes",
        ExecutionMode::InteractivePty => "interactive_pty",
    }
}

fn governance_capability_name(capability: &GovernanceCapability) -> &'static str {
    match capability {
        GovernanceCapability::FullyEnforced => "fully_enforced",
        GovernanceCapability::PartiallyEnforced => "partially_enforced",
        GovernanceCapability::ObservableOnly => "observable_only",
        GovernanceCapability::Unavailable => "unavailable",
    }
}

fn is_observability_degraded(governance: &GovernanceSnapshot) -> bool {
    !matches!(governance.capability, GovernanceCapability::FullyEnforced)
        || !governance.enforcement_gaps.is_empty()
}

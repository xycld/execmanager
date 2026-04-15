use std::fs::OpenOptions;
use std::io::{Seek, SeekFrom, Write};

use execmanager_contracts::{ExecutionId, ProjectionState};
use execmanager_daemon::{
    ExecutionMode, GhostView, Journal, JournalEvent, LaunchPolicyOutcome, PortView,
    RuntimeOwnership, RuntimeProjection, ServiceView,
};
use execmanager_platform::{
    EnforcementGap, GovernanceCapability, GovernancePlatform, GovernanceSnapshot, PlacementState,
    ResourceMetrics, ResourceProfile,
};
use execmanager_tui::{render_screen, PaneUiState, PrimaryView};
use tempfile::tempdir;

fn linux_governance_snapshot() -> GovernanceSnapshot {
    GovernanceSnapshot {
        platform: GovernancePlatform::Linux,
        capability: GovernanceCapability::FullyEnforced,
        profile: ResourceProfile {
            memory_max_bytes: Some(268_435_456),
            cpu_max_micros: Some(50_000),
            cpu_period_micros: Some(100_000),
        },
        placement: PlacementState::Applied {
            target: "/sys/fs/cgroup/execmanager/exec-tui-service-001".to_string(),
        },
        current: ResourceMetrics {
            memory_current_bytes: Some(65_536),
            memory_peak_bytes: Some(131_072),
            cpu_usage_micros: Some(5_000),
        },
        recent_peak: ResourceMetrics {
            memory_current_bytes: None,
            memory_peak_bytes: Some(131_072),
            cpu_usage_micros: Some(9_000),
        },
        enforcement_triggered: false,
        enforcement_gaps: Vec::new(),
    }
}

fn degraded_governance_snapshot() -> GovernanceSnapshot {
    GovernanceSnapshot {
        platform: GovernancePlatform::MacOs,
        capability: GovernanceCapability::ObservableOnly,
        profile: ResourceProfile::default(),
        placement: PlacementState::NotApplicable {
            reason: "macOS has no cgroup placement path".to_string(),
        },
        current: ResourceMetrics {
            memory_current_bytes: Some(20_480),
            memory_peak_bytes: Some(40_960),
            cpu_usage_micros: Some(1_024),
        },
        recent_peak: ResourceMetrics {
            memory_current_bytes: None,
            memory_peak_bytes: Some(40_960),
            cpu_usage_micros: Some(2_048),
        },
        enforcement_triggered: false,
        enforcement_gaps: vec![EnforcementGap {
            scope: "macos_degraded".to_string(),
            reason: "degraded: macOS has no cgroup placement path; degraded governance must stay explicit"
                .to_string(),
        }],
    }
}

fn render_state_for(exec_id: &ExecutionId, projection: &RuntimeProjection) -> String {
    render_screen(
        projection,
        &PaneUiState {
            selected_exec_id: Some(exec_id.clone()),
            focused_view: PrimaryView::InstanceDetail,
            ..PaneUiState::default()
        },
    )
    .expect("render should succeed")
}

#[test]
fn tui_displays_managed_service_lifecycle() {
    let temp = tempdir().expect("tempdir");
    let journal_path = temp.path().join("events.journal");
    let mut journal = Journal::open(&journal_path).expect("open journal");
    let exec_id = ExecutionId::new("exec-tui-service-001");

    for event in [
        JournalEvent::LaunchRequested {
            exec_id: exec_id.clone(),
            original_command: "npm run dev".to_string(),
            mode: ExecutionMode::BatchPipes,
        },
        JournalEvent::LaunchPolicyEvaluated {
            exec_id: exec_id.clone(),
            rewritten_command: Some("env PORT=4173 npm run dev".to_string()),
            outcome: LaunchPolicyOutcome::Rewritten {
                policy: "port_assignment".to_string(),
                reason: "launch spec was rewritten to pin the observed listener port".to_string(),
            },
        },
        JournalEvent::LaunchAdmitted {
            exec_id: exec_id.clone(),
            mode: ExecutionMode::BatchPipes,
        },
        JournalEvent::ProcessSpawned {
            exec_id: exec_id.clone(),
            state: ProjectionState::Managed,
            mode: ExecutionMode::BatchPipes,
            ownership: RuntimeOwnership {
                root_pid: 4242,
                process_group_id: 4242,
                session_id: Some(4242),
                start_time_ticks: Some(9001),
            },
            stdout: None,
            stderr: None,
        },
        JournalEvent::ResourceGovernanceRecorded {
            exec_id: exec_id.clone(),
            snapshot: linux_governance_snapshot(),
        },
        JournalEvent::ServiceObserved {
            exec_id: exec_id.clone(),
            service: ServiceView {
                name: "observed_service".to_string(),
                state: ProjectionState::Service,
                port_ids: vec!["tcp:4173".to_string()],
            },
        },
        JournalEvent::PortObserved {
            exec_id: exec_id.clone(),
            port: PortView {
                port_id: "tcp:4173".to_string(),
                port: 4173,
                protocol: "tcp".to_string(),
                state: ProjectionState::Service,
            },
        },
    ] {
        journal.append(&event).expect("append journal event");
    }

    let projection = RuntimeProjection::replay_from_path(&journal_path).expect("replay journal");
    let rendered = render_state_for(&exec_id, &projection);

    for expected in [
        "Instances",
        "Services",
        "History",
        "Ghosts/Reconcile",
        "Instance Detail",
        "exec-tui-service-001",
        "effective state: service",
        "original command: npm run dev",
        "rewritten launch spec: env PORT=4173 npm run dev",
        "policy: rewritten by port_assignment",
        "runtime state: service (observed service)",
        "resource governance: fully_enforced",
        "current memory: 65536",
        "recent peak memory: 131072",
        "service observed_service -> tcp:4173",
        "launch_requested",
        "launch_policy_evaluated",
        "process_spawned",
        "resource_governance_recorded",
        "service_observed",
        "port_observed",
    ] {
        assert!(
            rendered.contains(expected),
            "rendered TUI should contain {expected:?}, got:\n{rendered}"
        );
    }
}

#[test]
fn degraded_state_is_visible() {
    let temp = tempdir().expect("tempdir");
    let journal_path = temp.path().join("events.journal");
    let mut journal = Journal::open(&journal_path).expect("open journal");
    let exec_id = ExecutionId::new("exec-tui-degraded-001");

    for event in [
        JournalEvent::LaunchRequested {
            exec_id: exec_id.clone(),
            original_command: "python -m http.server".to_string(),
            mode: ExecutionMode::BatchPipes,
        },
        JournalEvent::LaunchPolicyEvaluated {
            exec_id: exec_id.clone(),
            rewritten_command: None,
            outcome: LaunchPolicyOutcome::AllowedAsRequested {
                policy: "default_allow".to_string(),
                reason: "no rewrite was required".to_string(),
            },
        },
        JournalEvent::LaunchAdmitted {
            exec_id: exec_id.clone(),
            mode: ExecutionMode::BatchPipes,
        },
        JournalEvent::ProcessSpawned {
            exec_id: exec_id.clone(),
            state: ProjectionState::Managed,
            mode: ExecutionMode::BatchPipes,
            ownership: RuntimeOwnership {
                root_pid: 5150,
                process_group_id: 5150,
                session_id: Some(5150),
                start_time_ticks: Some(77),
            },
            stdout: None,
            stderr: None,
        },
        JournalEvent::ResourceGovernanceRecorded {
            exec_id: exec_id.clone(),
            snapshot: degraded_governance_snapshot(),
        },
        JournalEvent::ExecutionStateUpdated {
            exec_id: exec_id.clone(),
            state: ProjectionState::Detached,
        },
        JournalEvent::GhostStateRecorded {
            exec_id: exec_id.clone(),
            ghost: GhostView {
                state: ProjectionState::Detached,
                detail:
                    "managed root process is gone but runtime artifacts were previously observed"
                        .to_string(),
            },
        },
        JournalEvent::PortObserved {
            exec_id: exec_id.clone(),
            port: PortView {
                port_id: "tcp:8000".to_string(),
                port: 8000,
                protocol: "tcp".to_string(),
                state: ProjectionState::Detached,
            },
        },
    ] {
        journal.append(&event).expect("append journal event");
    }

    let mut file = OpenOptions::new()
        .read(true)
        .write(true)
        .open(&journal_path)
        .expect("open file for corruption");
    file.seek(SeekFrom::End(-1)).expect("seek to file tail");
    file.write_all(&[0xAA]).expect("overwrite tail byte");
    file.flush().expect("flush corruption");

    let projection = RuntimeProjection::replay_with_degraded_state(&journal_path)
        .expect("degraded replay should still return projection");
    let rendered = render_state_for(&exec_id, &projection);

    for expected in [
        "Instances",
        "Services",
        "History",
        "Ghosts/Reconcile",
        "Instance Detail",
        "exec-tui-degraded-001",
        "replay degraded: journal corruption at offset",
        "resource governance: observable_only",
        "degraded: macOS has no cgroup placement path; degraded governance must stay explicit",
        "runtime state: detached",
        "ghost detached: managed root process is gone but runtime artifacts were previously observed",
        "policy: allowed_as_requested by default_allow",
        "observability degraded",
    ] {
        assert!(
            rendered.contains(expected),
            "rendered degraded TUI should contain {expected:?}, got:\n{rendered}"
        );
    }
}

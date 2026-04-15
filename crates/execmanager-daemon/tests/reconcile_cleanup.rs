use execmanager_contracts::{ExecutionId, ProjectionState};
use execmanager_daemon::{
    ExecutionMode, GhostView, Journal, JournalEvent, ObservedProcess, OwnershipError,
    RuntimeOwnership, RuntimeProjection, ServiceView,
};
use tempfile::tempdir;

fn append_spawned_execution(
    journal: &mut Journal,
    exec_id: &ExecutionId,
    ownership: RuntimeOwnership,
) {
    journal
        .append(&JournalEvent::LaunchRequested {
            exec_id: exec_id.clone(),
            original_command: "/bin/sleep 30".to_string(),
            mode: ExecutionMode::BatchPipes,
        })
        .expect("append launch requested");
    journal
        .append(&JournalEvent::LaunchAdmitted {
            exec_id: exec_id.clone(),
            mode: ExecutionMode::BatchPipes,
        })
        .expect("append launch admitted");
    journal
        .append(&JournalEvent::ProcessSpawned {
            exec_id: exec_id.clone(),
            state: ProjectionState::Managed,
            mode: ExecutionMode::BatchPipes,
            ownership,
            stdout: None,
            stderr: None,
        })
        .expect("append process spawned");
}

#[test]
fn restart_reconciliation_recovers_managed_state() {
    let temp = tempdir().expect("tempdir");
    let journal_path = temp.path().join("events.journal");
    let mut journal = Journal::open(&journal_path).expect("open journal");
    let exec_id = ExecutionId::new("exec-reconcile-001");
    let ownership = RuntimeOwnership {
        root_pid: 4242,
        process_group_id: 4242,
        session_id: Some(4242),
        start_time_ticks: Some(9001),
    };
    append_spawned_execution(&mut journal, &exec_id, ownership.clone());
    journal
        .append(&JournalEvent::ServiceObserved {
            exec_id: exec_id.clone(),
            service: ServiceView {
                name: "observed_service".to_string(),
                state: ProjectionState::Service,
                port_ids: vec!["tcp:8080".to_string()],
            },
        })
        .expect("append service observed");

    let mut projection =
        RuntimeProjection::replay_from_path(&journal_path).expect("replay journal");
    let summary = projection.reconcile_with_observed_processes(&[ObservedProcess {
        root_pid: 4242,
        process_group_id: 4242,
        session_id: Some(4242),
        start_time_ticks: Some(9001),
    }]);

    assert!(summary.unknown_processes.is_empty());

    let execution = projection
        .execution(exec_id.as_str())
        .expect("execution projection exists");
    assert_eq!(execution.state, ProjectionState::Service);
    assert_eq!(execution.observed_state, ProjectionState::Service);
    assert_eq!(execution.ownership.as_ref(), Some(&ownership));
    assert!(projection.ghost(exec_id.as_str()).is_none());
    assert_eq!(
        projection
            .cleanup_target(&exec_id)
            .expect("managed execution should remain cleanup-eligible"),
        ownership
            .cleanup_target()
            .expect("ownership proof is complete")
    );
}

#[test]
fn unknown_process_is_not_cleaned_up() {
    let temp = tempdir().expect("tempdir");
    let journal_path = temp.path().join("events.journal");
    let mut journal = Journal::open(&journal_path).expect("open journal");
    let exec_id = ExecutionId::new("exec-reconcile-unknown-001");
    append_spawned_execution(
        &mut journal,
        &exec_id,
        RuntimeOwnership {
            root_pid: 5150,
            process_group_id: 5150,
            session_id: Some(5150),
            start_time_ticks: Some(77),
        },
    );

    let mut projection =
        RuntimeProjection::replay_from_path(&journal_path).expect("replay journal");
    let summary = projection.reconcile_with_observed_processes(&[ObservedProcess {
        root_pid: 5150,
        process_group_id: 5150,
        session_id: Some(5150),
        start_time_ticks: Some(88),
    }]);

    assert_eq!(
        summary.unknown_processes,
        vec![ObservedProcess {
            root_pid: 5150,
            process_group_id: 5150,
            session_id: Some(5150),
            start_time_ticks: Some(88),
        }]
    );

    let execution = projection
        .execution(exec_id.as_str())
        .expect("execution projection exists");
    assert_eq!(execution.state, ProjectionState::Escaped);
    assert_eq!(execution.observed_state, ProjectionState::Escaped);
    assert_eq!(
        projection.ghost(exec_id.as_str()),
        Some(&GhostView {
            state: ProjectionState::Escaped,
            detail: "observed process did not match recorded ownership proof".to_string(),
        })
    );
    assert_eq!(
        projection
            .cleanup_target(&exec_id)
            .expect_err("escaped process must not be cleaned up"),
        OwnershipError::InsufficientProof {
            exec_id: Some(exec_id),
            reason: "execution is in uncertain reconciled state escaped".to_string(),
        }
    );
}

#[test]
fn unknown_execution_cleanup_is_refused() {
    let temp = tempdir().expect("tempdir");
    let journal_path = temp.path().join("events.journal");
    Journal::open(&journal_path).expect("create empty journal");
    let projection = RuntimeProjection::replay_from_path(&journal_path)
        .expect("empty journal should still yield projection");
    let exec_id = ExecutionId::new("exec-reconcile-absent-001");

    assert_eq!(
        projection
            .cleanup_target(&exec_id)
            .expect_err("cleanup must refuse unknown executions"),
        OwnershipError::InsufficientProof {
            exec_id: Some(exec_id),
            reason: "execution is not present in runtime projection".to_string(),
        }
    );
}

#[test]
fn reconciliation_is_idempotent_for_missing_execution() {
    let temp = tempdir().expect("tempdir");
    let journal_path = temp.path().join("events.journal");
    let mut journal = Journal::open(&journal_path).expect("open journal");
    let exec_id = ExecutionId::new("exec-reconcile-missing-001");
    append_spawned_execution(
        &mut journal,
        &exec_id,
        RuntimeOwnership {
            root_pid: 8080,
            process_group_id: 8080,
            session_id: Some(8080),
            start_time_ticks: Some(101),
        },
    );

    let mut projection =
        RuntimeProjection::replay_from_path(&journal_path).expect("replay journal");
    let first = projection.reconcile_with_observed_processes(&[]);
    let first_execution = projection
        .execution(exec_id.as_str())
        .expect("execution projection exists")
        .clone();
    let first_ghost = projection.ghost(exec_id.as_str()).cloned();

    let second = projection.reconcile_with_observed_processes(&[]);
    let second_execution = projection
        .execution(exec_id.as_str())
        .expect("execution projection exists")
        .clone();
    let second_ghost = projection.ghost(exec_id.as_str()).cloned();

    assert!(first.unknown_processes.is_empty());
    assert!(second.unknown_processes.is_empty());
    assert_eq!(first_execution, second_execution);
    assert_eq!(
        second_execution.state,
        ProjectionState::Missing,
        "reconciliation should mark absent managed roots explicitly"
    );
    assert_eq!(first_ghost, second_ghost);
}

#[test]
fn detached_service_without_live_root_is_marked_detached() {
    let temp = tempdir().expect("tempdir");
    let journal_path = temp.path().join("events.journal");
    let mut journal = Journal::open(&journal_path).expect("open journal");
    let exec_id = ExecutionId::new("exec-reconcile-detached-001");
    append_spawned_execution(
        &mut journal,
        &exec_id,
        RuntimeOwnership {
            root_pid: 9090,
            process_group_id: 9090,
            session_id: Some(9090),
            start_time_ticks: Some(202),
        },
    );
    journal
        .append(&JournalEvent::ServiceObserved {
            exec_id: exec_id.clone(),
            service: ServiceView {
                name: "observed_service".to_string(),
                state: ProjectionState::Service,
                port_ids: vec!["tcp:3000".to_string()],
            },
        })
        .expect("append service observed");

    let mut projection =
        RuntimeProjection::replay_from_path(&journal_path).expect("replay journal");
    let summary = projection.reconcile_with_observed_processes(&[]);

    assert!(summary.unknown_processes.is_empty());
    let execution = projection
        .execution(exec_id.as_str())
        .expect("execution projection exists");
    assert_eq!(execution.state, ProjectionState::Detached);
    assert_eq!(execution.observed_state, ProjectionState::Detached);
    assert_eq!(
        projection.ghost(exec_id.as_str()),
        Some(&GhostView {
            state: ProjectionState::Detached,
            detail: "managed root process is gone but runtime artifacts were previously observed"
                .to_string(),
        })
    );
}

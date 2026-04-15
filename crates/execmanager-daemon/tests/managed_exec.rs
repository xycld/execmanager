use std::path::PathBuf;

use execmanager_contracts::ExecutionId;
use execmanager_daemon::{
    ExecutionMode, LifecycleStage, ManagedExecError, ManagedExecutor, ManagedLaunchSpec,
    OwnershipError, RuntimeOwnership, RuntimeProjection,
};
use execmanager_platform::{
    GovernanceCapability, GovernanceCoordinator, GovernanceEnvironment, GovernancePlatform,
    PlacementState,
};
use tempfile::tempdir;

fn echo_program() -> PathBuf {
    PathBuf::from("/bin/echo")
}

#[test]
fn managed_batch_exec_lifecycle_works() {
    let temp = tempdir().expect("tempdir");
    let journal_path = temp.path().join("events.journal");
    let mut executor = ManagedExecutor::new(&journal_path).expect("create managed executor");
    let exec_id = ExecutionId::new("exec-managed-batch-001");

    let managed = executor
        .launch(ManagedLaunchSpec::new(
            exec_id.clone(),
            echo_program(),
            vec!["managed-batch".to_string()],
            ExecutionMode::BatchPipes,
        ))
        .expect("batch exec should be admitted and spawned");

    let ownership = managed.ownership().clone();
    assert!(ownership.root_pid > 0);
    assert!(ownership.process_group_id > 0);
    #[cfg(target_os = "linux")]
    assert!(ownership.start_time_ticks.is_some());

    let output = managed.wait_with_output().expect("batch exec output");
    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout), "managed-batch\n");

    let projection = RuntimeProjection::replay_from_path(&journal_path).expect("replay journal");
    let execution = projection
        .execution(exec_id.as_str())
        .expect("execution projection exists");

    assert_eq!(execution.exec_id.as_str(), exec_id.as_str());
    assert_eq!(execution.mode, Some(ExecutionMode::BatchPipes));
    assert_eq!(
        execution.lifecycle,
        vec![
            LifecycleStage::Requested,
            LifecycleStage::Admitted,
            LifecycleStage::Spawned,
            LifecycleStage::Exited,
        ]
    );
    assert_eq!(execution.ownership.as_ref(), Some(&ownership));
    assert_eq!(execution.command, "/bin/echo managed-batch");
}

#[test]
fn interactive_exec_without_pty_is_explicitly_rejected() {
    let temp = tempdir().expect("tempdir");
    let journal_path = temp.path().join("events.journal");
    let mut executor = ManagedExecutor::new(&journal_path).expect("create managed executor");

    let error = executor
        .launch(ManagedLaunchSpec::new(
            ExecutionId::new("exec-managed-pty-001"),
            echo_program(),
            vec!["interactive".to_string()],
            ExecutionMode::InteractivePty,
        ))
        .expect_err("interactive mode without PTY support must be rejected");

    match error {
        ManagedExecError::UnsupportedExecutionMode { requested, reason } => {
            assert_eq!(requested, ExecutionMode::InteractivePty);
            assert!(reason.contains("PTY"));
        }
        other => panic!("expected unsupported execution mode error, got {other:?}"),
    }

    let projection = RuntimeProjection::replay_from_path(&journal_path).expect("replay journal");
    assert!(projection.execution("exec-managed-pty-001").is_none());
}

#[test]
fn cleanup_target_requires_positive_ownership_proof() {
    let error = RuntimeOwnership {
        root_pid: 4242,
        process_group_id: 0,
        session_id: None,
        start_time_ticks: Some(99),
    }
    .cleanup_target()
    .expect_err("cleanup must refuse records without managed group proof");

    assert_eq!(
        error,
        OwnershipError::InsufficientProof {
            exec_id: None,
            reason: "managed process group id is missing".to_string(),
        }
    );
}

#[test]
fn managed_launch_records_resource_governance_state() {
    let temp = tempdir().expect("tempdir");
    let journal_path = temp.path().join("events.journal");
    let cgroup_root = temp.path().join("cgroup2");
    std::fs::create_dir_all(&cgroup_root).expect("create cgroup root");
    std::fs::write(cgroup_root.join("cgroup.controllers"), "cpu memory")
        .expect("write controllers");

    let mut executor = ManagedExecutor::new_with_governance(
        &journal_path,
        GovernanceCoordinator::for_environment(GovernanceEnvironment::linux_for_tests(
            &cgroup_root,
        )),
    )
    .expect("create managed executor");
    let exec_id = ExecutionId::new("exec-managed-governance-001");

    let managed = executor
        .launch(ManagedLaunchSpec::new(
            exec_id.clone(),
            echo_program(),
            vec!["managed-governance".to_string()],
            ExecutionMode::BatchPipes,
        ))
        .expect("batch exec should be admitted and spawned");

    let output = managed.wait_with_output().expect("batch exec output");
    assert!(output.status.success());

    let projection = RuntimeProjection::replay_from_path(&journal_path).expect("replay journal");
    let execution = projection
        .execution(exec_id.as_str())
        .expect("execution projection exists");

    let governance = execution
        .resource_governance
        .as_ref()
        .expect("resource governance must be projection-visible");
    assert_eq!(governance.platform, GovernancePlatform::Linux);
    assert_eq!(governance.capability, GovernanceCapability::ObservableOnly);
    assert!(matches!(
        governance.placement,
        PlacementState::Applied { .. }
    ));
    assert!(governance.enforcement_gaps.is_empty());
}

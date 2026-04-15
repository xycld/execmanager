use std::env;
use std::fs;
use std::path::PathBuf;

use execmanager_contracts::ExecutionId;
use execmanager_daemon::{
    ExecutionMode, LaunchPolicyOutcome, ManagedExecError, ManagedExecutor, ManagedLaunchSpec,
    RuntimeProjection,
};
use tempfile::tempdir;

fn rm_program() -> PathBuf {
    PathBuf::from("/bin/rm")
}

#[test]
fn direct_rm_is_rewritten_to_safe_delete() {
    let temp = tempdir().expect("tempdir");
    let journal_path = temp.path().join("events.journal");
    let victim = temp.path().join("delete-me.txt");
    fs::write(&victim, b"important contents").expect("write victim file");

    let mut executor = ManagedExecutor::new(&journal_path).expect("create managed executor");
    let exec_id = ExecutionId::new("exec-rm-rewrite-001");

    let managed = executor
        .launch(ManagedLaunchSpec::new(
            exec_id.clone(),
            rm_program(),
            vec![victim.display().to_string()],
            ExecutionMode::BatchPipes,
        ))
        .expect("direct rm should be rewritten to safe delete backend");

    let output = managed
        .wait_with_output()
        .expect("wait for rewritten safe delete");
    assert!(output.status.success());
    assert!(
        !victim.exists(),
        "victim should be removed from original path"
    );

    let trash_dir = temp.path().join(".execmanager-trash");
    assert!(
        trash_dir.is_dir(),
        "trash directory should be created next to operand"
    );

    let trashed_entries: Vec<_> = fs::read_dir(&trash_dir)
        .expect("read trash dir")
        .map(|entry| entry.expect("trash entry").path())
        .collect();
    assert_eq!(
        trashed_entries.len(),
        1,
        "exactly one item should be moved to trash"
    );
    assert_eq!(
        fs::read(&trashed_entries[0]).expect("read trashed file"),
        b"important contents"
    );

    let projection = RuntimeProjection::replay_from_path(&journal_path).expect("replay journal");
    let execution = projection
        .execution(exec_id.as_str())
        .expect("execution projection exists");

    assert_eq!(
        execution.original_command,
        format!("/bin/rm {}", victim.display())
    );
    assert!(execution
        .rewritten_command
        .as_ref()
        .expect("rewritten command")
        .starts_with(&format!("/bin/mv -- {} ", victim.display())));
    assert_eq!(
        execution.policy_outcome,
        Some(LaunchPolicyOutcome::Rewritten {
            policy: "rm_safety_adapter".to_string(),
            reason: "direct rm operand was deterministically rewritten to safe delete".to_string(),
        })
    );
}

#[test]
fn protected_path_rm_is_blocked() {
    let temp = tempdir().expect("tempdir");
    let journal_path = temp.path().join("events.journal");
    let home_dir = PathBuf::from(env::var("HOME").expect("HOME environment variable"));

    let mut executor = ManagedExecutor::new(&journal_path).expect("create managed executor");
    let exec_id = ExecutionId::new("exec-rm-protected-001");

    let error = executor
        .launch(ManagedLaunchSpec::new(
            exec_id.clone(),
            rm_program(),
            vec![home_dir.display().to_string()],
            ExecutionMode::BatchPipes,
        ))
        .expect_err("protected rm should be denied before spawn");

    assert_eq!(
        error,
        ManagedExecError::PolicyDenied {
            policy: "rm_safety_adapter".to_string(),
            reason: format!(
                "resolved operand {} targets a protected path",
                home_dir.display()
            ),
        }
    );

    let projection = RuntimeProjection::replay_from_path(&journal_path).expect("replay journal");
    let execution = projection
        .execution(exec_id.as_str())
        .expect("blocked execution should still be auditable");

    assert_eq!(
        execution.original_command,
        format!("/bin/rm {}", home_dir.display())
    );
    assert_eq!(execution.rewritten_command, None);
    assert_eq!(
        execution.policy_outcome,
        Some(LaunchPolicyOutcome::Denied {
            policy: "rm_safety_adapter".to_string(),
            reason: format!(
                "resolved operand {} targets a protected path",
                home_dir.display()
            ),
        })
    );
}

#[test]
fn ambiguous_form_rm_is_blocked() {
    let temp = tempdir().expect("tempdir");
    let journal_path = temp.path().join("events.journal");

    let mut executor = ManagedExecutor::new(&journal_path).expect("create managed executor");
    let exec_id = ExecutionId::new("exec-rm-ambiguous-001");

    let error = executor
        .launch(ManagedLaunchSpec::new(
            exec_id.clone(),
            rm_program(),
            vec!["*.log".to_string()],
            ExecutionMode::BatchPipes,
        ))
        .expect_err("ambiguous rm should be denied before spawn");

    assert_eq!(
        error,
        ManagedExecError::PolicyDenied {
            policy: "rm_safety_adapter".to_string(),
            reason: "operand *.log contains ambiguous shell metacharacters".to_string(),
        }
    );

    let projection = RuntimeProjection::replay_from_path(&journal_path).expect("replay journal");
    let execution = projection
        .execution(exec_id.as_str())
        .expect("blocked execution should still be auditable");

    assert_eq!(execution.original_command, "/bin/rm *.log");
    assert_eq!(execution.rewritten_command, None);
    assert_eq!(
        execution.policy_outcome,
        Some(LaunchPolicyOutcome::Denied {
            policy: "rm_safety_adapter".to_string(),
            reason: "operand *.log contains ambiguous shell metacharacters".to_string(),
        })
    );
}

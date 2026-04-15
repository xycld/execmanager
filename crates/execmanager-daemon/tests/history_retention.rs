use std::path::PathBuf;

use execmanager_contracts::{ExecutionId, ProjectionState, RedactionMarker, RetentionClass};
use execmanager_daemon::{ExecutionMode, ManagedExecutor, ManagedLaunchSpec, RuntimeProjection};
use tempfile::tempdir;

fn shell_program() -> PathBuf {
    PathBuf::from("/bin/sh")
}

#[test]
fn secrets_are_redacted_in_persisted_records() {
    let temp = tempdir().expect("tempdir");
    let journal_path = temp.path().join("events.journal");
    let mut executor = ManagedExecutor::new(&journal_path).expect("create managed executor");
    let exec_id = ExecutionId::new("exec-history-redaction-001");

    let managed = executor
        .launch(
            ManagedLaunchSpec::new(
                exec_id.clone(),
                shell_program(),
                vec!["-c".to_string(), "printf retained-output".to_string()],
                ExecutionMode::BatchPipes,
            )
            .with_env("APP_ENV", "test")
            .with_env("API_TOKEN", "super-secret-token")
            .with_original_command(
                "API_TOKEN=super-secret-token /bin/sh -c printf retained-output",
            ),
        )
        .expect("launch managed execution");

    let output = managed.wait_with_output().expect("wait for output");
    assert!(output.status.success());

    let projection = RuntimeProjection::replay_from_path(&journal_path).expect("replay journal");
    let execution = projection
        .execution(exec_id.as_str())
        .expect("execution projection exists");
    let env_snapshot = projection
        .env_snapshot(exec_id.as_str())
        .expect("env snapshot should be persisted");

    assert_eq!(env_snapshot.retention, RetentionClass::MetadataLongLived);
    assert_eq!(env_snapshot.redaction, RedactionMarker::Redacted);
    assert!(env_snapshot.entries.iter().any(|entry| {
        entry.name == "APP_ENV"
            && entry.value.as_deref() == Some("test")
            && entry.value_redaction == RedactionMarker::Plaintext
    }));
    assert!(env_snapshot.entries.iter().any(|entry| {
        entry.name_redaction == RedactionMarker::Redacted
            && entry.value_redaction == RedactionMarker::Omitted
    }));
    let persisted_env_json = serde_json::to_string(env_snapshot).expect("serialize env snapshot");
    assert!(!persisted_env_json.contains("API_TOKEN"));
    assert!(!persisted_env_json.contains("super-secret-token"));

    assert!(!execution.original_command.contains("super-secret-token"));

    let manifest = projection
        .history_manifest(exec_id.as_str())
        .expect("history manifest should be present");
    assert_eq!(manifest.schema_version, 1);
    assert_eq!(manifest.retention, RetentionClass::MetadataLongLived);
    assert_eq!(manifest.execution.exec_id, exec_id);
    assert_eq!(manifest.environment.record_id, env_snapshot.record_id);
    assert_eq!(manifest.environment.redaction, RedactionMarker::Redacted);
    assert_eq!(manifest.observed.exit_code, Some(0));
    assert_eq!(manifest.observed.final_state, ProjectionState::Exited);

    let history = projection.history().expect("load history");
    assert!(history
        .iter()
        .all(|record| record.retention == RetentionClass::MetadataLongLived));
}

#[test]
fn high_volume_output_does_not_deadlock() {
    let temp = tempdir().expect("tempdir");
    let journal_path = temp.path().join("events.journal");
    let mut executor = ManagedExecutor::new(&journal_path).expect("create managed executor");
    let exec_id = ExecutionId::new("exec-history-output-001");
    let script = "dd if=/dev/zero bs=65536 count=40 status=none | tr '\\000' 'o'; dd if=/dev/zero bs=65536 count=40 status=none | tr '\\000' 'e' 1>&2";

    let managed = executor
        .launch(ManagedLaunchSpec::new(
            exec_id.clone(),
            shell_program(),
            vec!["-c".to_string(), script.to_string()],
            ExecutionMode::BatchPipes,
        ))
        .expect("launch managed execution");

    let output = managed
        .wait_with_output()
        .expect("high-volume output should complete");
    assert!(output.status.success());

    let projection = RuntimeProjection::replay_from_path(&journal_path).expect("replay journal");
    let execution = projection
        .execution(exec_id.as_str())
        .expect("execution projection exists");
    let stdout = execution
        .stdout
        .as_ref()
        .expect("stdout blob ref should exist");
    let stderr = execution
        .stderr
        .as_ref()
        .expect("stderr blob ref should exist");

    let manifest = projection
        .history_manifest(exec_id.as_str())
        .expect("history manifest should be present");
    let stdout_artifact = manifest
        .artifacts
        .stdout
        .as_ref()
        .expect("stdout artifact should exist");
    let stderr_artifact = manifest
        .artifacts
        .stderr
        .as_ref()
        .expect("stderr artifact should exist");

    assert_eq!(stdout_artifact.retention, RetentionClass::BlobEphemeral);
    assert_eq!(stderr_artifact.retention, RetentionClass::BlobEphemeral);
    assert!(stdout.size_bytes >= 2_621_440);
    assert!(stderr.size_bytes >= 2_621_440);
    assert!(stdout_artifact.truncated);
    assert!(stderr_artifact.truncated);
    assert!(
        std::fs::metadata(&stdout.storage_path)
            .expect("stdout spool file exists")
            .len()
            >= stdout.size_bytes
    );
    assert!(
        std::fs::metadata(&stderr.storage_path)
            .expect("stderr spool file exists")
            .len()
            >= stderr.size_bytes
    );
    assert!(output.stdout.len() < stdout.size_bytes as usize);
    assert!(output.stderr.len() < stderr.size_bytes as usize);

    assert_eq!(
        manifest
            .artifacts
            .stdout
            .as_ref()
            .map(|artifact| &artifact.blob),
        Some(stdout)
    );
    assert_eq!(
        manifest
            .artifacts
            .stderr
            .as_ref()
            .map(|artifact| &artifact.blob),
        Some(stderr)
    );
}

#[test]
fn snapshot_manifest_is_future_restore_ready_without_restore_execution() {
    let temp = tempdir().expect("tempdir");
    let journal_path = temp.path().join("events.journal");
    let mut executor = ManagedExecutor::new(&journal_path).expect("create managed executor");
    let exec_id = ExecutionId::new("exec-history-manifest-001");

    let managed = executor
        .launch(
            ManagedLaunchSpec::new(
                exec_id.clone(),
                shell_program(),
                vec!["-c".to_string(), "printf restore-ready".to_string()],
                ExecutionMode::BatchPipes,
            )
            .with_env("SAFE_NAME", "visible"),
        )
        .expect("launch managed execution");

    let output = managed.wait_with_output().expect("wait for output");
    assert!(output.status.success());

    let projection = RuntimeProjection::replay_from_path(&journal_path).expect("replay journal");
    let execution = projection
        .execution(exec_id.as_str())
        .expect("execution projection exists");
    let manifest = projection
        .history_manifest(exec_id.as_str())
        .expect("history manifest should be present");

    assert_eq!(manifest.schema_version, 1);
    assert!(!manifest.snapshot_id.is_empty());
    assert!(manifest.captured_at_epoch_ms > 0);
    assert_eq!(manifest.execution.exec_id, exec_id);
    assert_eq!(
        manifest.intent.original_command,
        "/bin/sh -c printf restore-ready"
    );
    assert_eq!(manifest.launch.rewritten_command, None);
    assert_eq!(manifest.observed.final_state, ProjectionState::Exited);
    assert_eq!(manifest.observed.exit_code, Some(0));
    assert!(manifest.environment.record_id.starts_with("env-"));
    assert_eq!(manifest.environment.redaction, RedactionMarker::Plaintext);
    assert_eq!(
        manifest
            .artifacts
            .stdout
            .as_ref()
            .map(|artifact| &artifact.blob),
        execution.stdout.as_ref()
    );
    assert_eq!(
        manifest
            .artifacts
            .stderr
            .as_ref()
            .map(|artifact| &artifact.blob),
        execution.stderr.as_ref()
    );
    assert_eq!(manifest.host.platform, std::env::consts::OS);
    assert_eq!(manifest.retention, RetentionClass::MetadataLongLived);
}

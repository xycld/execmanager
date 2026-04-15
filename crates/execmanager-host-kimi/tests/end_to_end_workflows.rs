use std::fs;
use std::net::TcpListener;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::Duration;

use execmanager_contracts::{
    ExecutionId, ProjectionState, ViewerRequest,
};
use execmanager_daemon::{spawn_rpc_server, DaemonRpcConfig, RuntimeProjection};
use execmanager_host_kimi::{
    route_tool_call, KimiToolCall, ManagedExecProof, ShellToolCall, ToolCallKind,
};
use execmanager_tui::{render_screen, PaneUiState, PrimaryView};
use execmanager_viewers::{
    attach_viewer, ViewerAdapter, ViewerAttachError, ViewerAttachment, ViewerOwnership,
};
use libc::{killpg, SIGTERM};
use tempfile::tempdir;
use tokio::time::sleep;

#[derive(Default)]
struct RecordingAdapter {
    attached_exec_ids: Vec<String>,
}

impl ViewerAdapter for RecordingAdapter {
    fn attach(&mut self, handle: &execmanager_daemon::ViewerHandle) -> Result<ViewerAttachment, ViewerAttachError> {
        self.attached_exec_ids
            .push(handle.exec_id.as_str().to_string());
        Ok(ViewerAttachment {
            exec_id: handle.exec_id.clone(),
            ownership: ViewerOwnership::AttachedReadOnly,
        })
    }
}

fn python3_program() -> PathBuf {
    PathBuf::from("/usr/bin/python3")
}

fn rm_program() -> PathBuf {
    PathBuf::from("/bin/rm")
}

fn reserve_local_port() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").expect("reserve local port");
    let port = listener.local_addr().expect("local addr").port();
    drop(listener);
    port
}

fn write_dev_server_script(path: &Path) {
    fs::write(
        path,
        r#"import socket
import sys
import time

port = int(sys.argv[1])
sock = socket.socket()
sock.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
sock.bind(("127.0.0.1", port))
sock.listen(8)
time.sleep(10)
"#,
    )
    .expect("write dev server script");
    let mut permissions = fs::metadata(path).expect("script metadata").permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(path, permissions).expect("chmod script");
}

fn selected_detail_view(exec_id: &ExecutionId, projection: &RuntimeProjection) -> String {
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

fn count_process_spawned(projection: &RuntimeProjection, exec_id: &ExecutionId) -> usize {
    projection
        .history()
        .expect("read journal history")
        .into_iter()
        .filter(|record| match &record.event {
            execmanager_daemon::JournalEvent::ProcessSpawned { exec_id: event_exec_id, .. } => {
                event_exec_id == exec_id
            }
            _ => false,
        })
        .count()
}

async fn wait_for_projection<F>(journal_path: &Path, mut predicate: F) -> RuntimeProjection
where
    F: FnMut(&RuntimeProjection) -> bool,
{
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    loop {
        if let Ok(projection) = RuntimeProjection::replay_from_path(journal_path) {
            if predicate(&projection) {
                return projection;
            }
        }

        assert!(
            std::time::Instant::now() < deadline,
            "timed out waiting for projection predicate on {}",
            journal_path.display()
        );
        sleep(Duration::from_millis(50)).await;
    }
}

fn terminate_managed_execution(projection: &RuntimeProjection, exec_id: &ExecutionId) {
    let target = projection
        .cleanup_target(exec_id)
        .expect("daemon projection should expose cleanup target");
    let result = unsafe { killpg(target.process_group_id as i32, SIGTERM) };
    assert_eq!(result, 0, "killpg should terminate managed process group");
}

#[tokio::test]
async fn managed_dev_server_appears_and_opens_viewer() {
    let temp = tempdir().expect("tempdir");
    let socket_path = temp.path().join("kimi.sock");
    let journal_path = temp.path().join("events.journal");
    let script_path = temp.path().join("dev_server.py");
    let port = reserve_local_port();
    write_dev_server_script(&script_path);
    let command = format!(
        "{} {} {}",
        python3_program().display(),
        script_path.display(),
        port
    );
    let server = spawn_rpc_server(DaemonRpcConfig::new(&socket_path, &journal_path))
        .expect("spawn daemon rpc server");

    let proof = route_tool_call(
        &socket_path,
        KimiToolCall {
            tool_name: "Shell".to_string(),
            kind: ToolCallKind::AgentIssuedShell(ShellToolCall {
                command: command.clone(),
            }),
        },
    )
    .await
    .expect("route tool call");

    let managed_launch = match proof {
        ManagedExecProof::Managed(launch) => launch,
        other => panic!("expected managed launch proof, got {other:?}"),
    };
    let exec_id = ExecutionId::new(managed_launch.exec_id.clone());
    assert!(managed_launch.pre_spawn);

    let projection = wait_for_projection(&journal_path, |projection| {
        projection
            .execution(exec_id.as_str())
            .map(|execution| execution.observed_state == ProjectionState::Service)
            .unwrap_or(false)
    })
    .await;
    let execution = projection
        .execution(exec_id.as_str())
        .expect("execution projection exists");
    assert_eq!(execution.observed_state, ProjectionState::Service);
    assert_eq!(execution.state, ProjectionState::Service);

    let service = projection
        .service(exec_id.as_str(), "observed_service")
        .expect("service projection exists");
    assert_eq!(service.state, ProjectionState::Service);
    assert_eq!(service.port_ids, vec![format!("tcp:{port}")]);

    let observed_port = projection
        .port(exec_id.as_str(), &format!("tcp:{port}"))
        .expect("port projection exists");
    assert_eq!(observed_port.port, port);

    let rendered = selected_detail_view(&exec_id, &projection);
    for expected in [
        exec_id.as_str(),
        "effective state: service",
        "runtime state: service (observed service)",
        &format!("original command: {command}"),
        &format!("service observed_service -> tcp:{port}"),
        "launch_requested",
        "process_spawned",
        "service_observed",
        "port_observed",
    ] {
        assert!(
            rendered.contains(expected),
            "rendered screen should contain {expected:?}, got:\n{rendered}"
        );
    }

    let handle = projection
        .viewer_handle(&exec_id)
        .expect("daemon projection should yield viewer handle");
    assert_eq!(handle.exec_id, exec_id);
    assert_eq!(handle.journal_path.as_deref(), Some(journal_path.as_path()));

    let mut adapter = RecordingAdapter::default();
    let attachment = attach_viewer(
        &ViewerRequest {
            exec_id: exec_id.clone(),
        },
        &handle,
        &mut adapter,
    )
    .expect("viewer should attach from daemon handle");
    assert_eq!(attachment.ownership, ViewerOwnership::AttachedReadOnly);
    assert_eq!(adapter.attached_exec_ids, vec![exec_id.as_str().to_string()]);

    terminate_managed_execution(&projection, &exec_id);
    let _ = wait_for_projection(&journal_path, |projection| {
        projection
            .execution(exec_id.as_str())
            .map(|execution| execution.state == ProjectionState::Exited)
            .unwrap_or(false)
    })
    .await;
    server.shutdown().await.expect("server shutdown");
}

#[tokio::test]
async fn viewer_open_does_not_emit_spawn() {
    let temp = tempdir().expect("tempdir");
    let socket_path = temp.path().join("kimi.sock");
    let journal_path = temp.path().join("events.journal");
    let script_path = temp.path().join("viewer_target.py");
    let port = reserve_local_port();
    write_dev_server_script(&script_path);
    let command = format!(
        "{} {} {}",
        python3_program().display(),
        script_path.display(),
        port
    );
    let server = spawn_rpc_server(DaemonRpcConfig::new(&socket_path, &journal_path))
        .expect("spawn daemon rpc server");

    let proof = route_tool_call(
        &socket_path,
        KimiToolCall {
            tool_name: "Shell".to_string(),
            kind: ToolCallKind::AgentIssuedShell(ShellToolCall {
                command: command.clone(),
            }),
        },
    )
    .await
    .expect("route tool call");

    let exec_id = match proof {
        ManagedExecProof::Managed(launch) => ExecutionId::new(launch.exec_id),
        other => panic!("expected managed launch proof, got {other:?}"),
    };

    let projection_before = wait_for_projection(&journal_path, |projection| {
        projection
            .execution(exec_id.as_str())
            .map(|execution| execution.observed_state == ProjectionState::Service)
            .unwrap_or(false)
    })
    .await;
    let spawn_count_before = count_process_spawned(&projection_before, &exec_id);
    assert_eq!(spawn_count_before, 1, "managed exec should spawn exactly once");

    let handle = projection_before
        .viewer_handle(&exec_id)
        .expect("projection should expose daemon viewer handle");
    let mut adapter = RecordingAdapter::default();
    let attachment = attach_viewer(
        &ViewerRequest {
            exec_id: exec_id.clone(),
        },
        &handle,
        &mut adapter,
    )
    .expect("viewer should attach");
    assert_eq!(attachment.exec_id, exec_id);

    let projection_after = RuntimeProjection::replay_from_path(&journal_path).expect("replay journal");
    let spawn_count_after = count_process_spawned(&projection_after, &exec_id);
    assert_eq!(spawn_count_after, 1, "viewer opening must not create a second spawn path");
    assert_eq!(adapter.attached_exec_ids, vec![exec_id.as_str().to_string()]);

    terminate_managed_execution(&projection_after, &exec_id);
    let _ = wait_for_projection(&journal_path, |projection| {
        projection
            .execution(exec_id.as_str())
            .map(|execution| execution.state == ProjectionState::Exited)
            .unwrap_or(false)
    })
    .await;
    server.shutdown().await.expect("server shutdown");
}

#[tokio::test]
async fn rm_safety_rewrite_is_visible_in_tui_history() {
    let temp = tempdir().expect("tempdir");
    let socket_path = temp.path().join("kimi.sock");
    let journal_path = temp.path().join("events.journal");
    let victim_path = temp.path().join("delete-me.txt");
    fs::write(&victim_path, b"keep me safe").expect("write victim file");
    let command = format!("{} {}", rm_program().display(), victim_path.display());
    let server = spawn_rpc_server(DaemonRpcConfig::new(&socket_path, &journal_path))
        .expect("spawn daemon rpc server");

    let proof = route_tool_call(
        &socket_path,
        KimiToolCall {
            tool_name: "Shell".to_string(),
            kind: ToolCallKind::AgentIssuedShell(ShellToolCall {
                command: command.clone(),
            }),
        },
    )
    .await
    .expect("route tool call");

    let exec_id = match proof {
        ManagedExecProof::Managed(launch) => ExecutionId::new(launch.exec_id),
        other => panic!("expected managed launch proof, got {other:?}"),
    };

    let projection = wait_for_projection(&journal_path, |projection| {
        projection
            .history_manifest(exec_id.as_str())
            .is_some()
    })
    .await;
    let execution = projection
        .execution(exec_id.as_str())
        .expect("execution projection exists");
    assert_eq!(execution.original_command, command);
    assert_eq!(
        execution.policy_outcome,
        Some(execmanager_daemon::LaunchPolicyOutcome::Rewritten {
            policy: "rm_safety_adapter".to_string(),
            reason: "direct rm operand was deterministically rewritten to safe delete".to_string(),
        })
    );
    assert!(
        execution
            .rewritten_command
            .as_deref()
            .expect("rewritten command exists")
            .starts_with(&format!("/bin/mv -- {} ", victim_path.display()))
    );

    let rendered = selected_detail_view(&exec_id, &projection);
    for expected in [
        &format!("original command: {}", execution.original_command),
        "policy: rewritten by rm_safety_adapter",
        "launch_requested",
        "launch_policy_evaluated",
        "history_snapshot_recorded",
    ] {
        assert!(
            rendered.contains(expected),
            "rendered screen should contain {expected:?}, got:\n{rendered}"
        );
    }

    assert!(!victim_path.exists(), "original rm target should be moved away");
    assert!(
        temp.path().join(".execmanager-trash").is_dir(),
        "safe-delete rewrite should produce trash directory"
    );

    server.shutdown().await.expect("server shutdown");
}

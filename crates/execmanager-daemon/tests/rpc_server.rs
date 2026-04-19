use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::Duration;

use execmanager_contracts::{
    DaemonRequestEnvelope, DaemonResponseEnvelope, HandshakeRejectReason, HandshakeRequest,
    HandshakeResponse, ProjectionState,
};
use execmanager_daemon::{spawn_rpc_server, DaemonRpcConfig, RuntimeProjection};
use futures::{SinkExt, StreamExt};
use tempfile::tempdir;
use tokio::net::UnixStream;
use tokio::time::sleep;
use tokio_util::codec::{Framed, LengthDelimitedCodec};

fn python3_program() -> PathBuf {
    PathBuf::from("/usr/bin/python3")
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
time.sleep(2)
"#,
    )
    .expect("write dev server script");
    let mut permissions = fs::metadata(path).expect("script metadata").permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(path, permissions).expect("chmod script");
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

        assert!(std::time::Instant::now() < deadline, "timed out waiting for projection");
        sleep(Duration::from_millis(50)).await;
    }
}

#[tokio::test]
async fn daemon_rpc_rejects_incompatible_protocol_versions() {
    let temp = tempdir().expect("tempdir");
    let socket_path = temp.path().join("daemon.sock");
    let journal_path = temp.path().join("events.journal");
    let server = spawn_rpc_server(DaemonRpcConfig::new(&socket_path, &journal_path))
        .expect("spawn daemon server");

    let stream = UnixStream::connect(&socket_path)
        .await
        .expect("connect daemon socket");
    let mut framed = Framed::new(stream, LengthDelimitedCodec::new());

    let mut request = HandshakeRequest::new("rpc-test");
    request.protocol_version += 1;
    let envelope = DaemonRequestEnvelope::Handshake(request);
    framed
        .send(
            serde_json::to_vec(&envelope)
                .expect("encode handshake request")
                .into(),
        )
        .await
        .expect("send handshake request");

    let frame = framed
        .next()
        .await
        .expect("response frame")
        .expect("response bytes");
    let response: DaemonResponseEnvelope =
        serde_json::from_slice(&frame).expect("decode response");

    match response {
        DaemonResponseEnvelope::Handshake(HandshakeResponse::Rejected(rejected)) => {
            assert!(matches!(
                rejected.reason,
                HandshakeRejectReason::IncompatibleProtocolVersion { .. }
            ));
        }
        other => panic!("expected rejected handshake, got {other:?}"),
    }

    server.shutdown().await.expect("server shutdown");
}

#[tokio::test]
async fn daemon_rpc_launches_and_projects_real_execution() {
    let temp = tempdir().expect("tempdir");
    let socket_path = temp.path().join("daemon.sock");
    let journal_path = temp.path().join("events.journal");
    let script_path = temp.path().join("short_task.py");
    write_dev_server_script(&script_path);
    let command = format!("{} {} {}", python3_program().display(), script_path.display(), 0);

    let server = spawn_rpc_server(DaemonRpcConfig::new(&socket_path, &journal_path))
        .expect("spawn daemon server");

    let stream = UnixStream::connect(&socket_path)
        .await
        .expect("connect daemon socket");
    let mut framed = Framed::new(stream, LengthDelimitedCodec::new());
    framed
        .send(
            serde_json::to_vec(&DaemonRequestEnvelope::Handshake(HandshakeRequest::new(
                "rpc-test",
            )))
            .expect("encode handshake")
            .into(),
        )
        .await
        .expect("send handshake");

    let handshake_frame = framed
        .next()
        .await
        .expect("handshake response frame")
        .expect("handshake response bytes");
    let handshake_response: DaemonResponseEnvelope =
        serde_json::from_slice(&handshake_frame).expect("decode handshake response");
    assert!(matches!(
        handshake_response,
        DaemonResponseEnvelope::Handshake(HandshakeResponse::Accepted(_))
    ));

    framed
        .send(
            serde_json::to_vec(&DaemonRequestEnvelope::Launch(execmanager_contracts::LaunchRequest {
                tool_name: "Shell".to_string(),
                command: command.clone(),
                working_dir: Some(temp.path().display().to_string()),
                source: Some("rpc-test:shell".to_string()),
            }))
            .expect("encode launch request")
            .into(),
        )
        .await
        .expect("send launch request");

    let launch_frame = framed
        .next()
        .await
        .expect("launch response frame")
        .expect("launch response bytes");
    let launch_response: DaemonResponseEnvelope =
        serde_json::from_slice(&launch_frame).expect("decode launch response");
    let exec_id = match launch_response {
        DaemonResponseEnvelope::Launch(response) => response.exec_id,
        other => panic!("expected launch response, got {other:?}"),
    };

    let projection = wait_for_projection(&journal_path, |projection| {
        projection
            .execution(exec_id.as_str())
            .map(|execution| execution.state == ProjectionState::Exited)
            .unwrap_or(false)
    })
    .await;

    let execution = projection
        .execution(exec_id.as_str())
        .expect("execution projection exists");
    assert_eq!(execution.exec_id, exec_id);
    assert_eq!(execution.original_command, command);
    assert_eq!(execution.state, ProjectionState::Exited);
    assert!(projection.history_manifest(exec_id.as_str()).is_some());
    let env_snapshot = projection
        .env_snapshot(exec_id.as_str())
        .expect("env snapshot exists");
    assert!(env_snapshot
        .entries
        .iter()
        .any(|entry| entry.name == "PWD" && entry.value.as_deref() == Some(&temp.path().display().to_string())));
    assert!(env_snapshot
        .entries
        .iter()
        .any(|entry| entry.name == "EXECMANAGER_SOURCE" && entry.value.as_deref() == Some("rpc-test:shell")));

    server.shutdown().await.expect("server shutdown");
}

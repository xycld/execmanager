use execmanager_contracts::{
    DaemonRequestEnvelope, DaemonResponseEnvelope, HandshakeRejectReason, HandshakeRequest,
    HandshakeResponse, PROTOCOL_VERSION,
};
use execmanager_daemon::{spawn_rpc_server, DaemonRpcConfig};
use execmanager_host_kimi::{
    route_tool_call, IngressError, KimiToolCall, ManagedExecProof, ShellToolCall, ToolCallKind,
};
use futures::{SinkExt, StreamExt};
use tempfile::tempdir;
use tokio::net::UnixStream;
use tokio_util::codec::{Framed, LengthDelimitedCodec};

#[tokio::test]
async fn supported_exec_routes_through_daemon() {
    let temp = tempdir().expect("tempdir");
    let socket_path = temp.path().join("kimi.sock");
    let journal_path = temp.path().join("events.journal");
    let server = spawn_rpc_server(DaemonRpcConfig::new(&socket_path, &journal_path))
        .expect("spawn daemon rpc server");

    let proof = route_tool_call(
        &socket_path,
        KimiToolCall {
            tool_name: "Shell".to_string(),
            kind: ToolCallKind::AgentIssuedShell(ShellToolCall {
                command: "echo managed".to_string(),
            }),
        },
    )
    .await
    .expect("supported shell ingress should be managed");

    match proof {
        ManagedExecProof::Managed(managed) => {
            assert!(!managed.exec_id.is_empty());
            assert_eq!(managed.command, "echo managed");
            assert!(managed.pre_spawn);
        }
        other => panic!("expected managed proof, got {other:?}"),
    }

    server.shutdown().await.expect("server shutdown");
}

#[tokio::test]
async fn unsupported_exec_is_marked_non_coverage() {
    let proof = route_tool_call(
        "/tmp/unused.sock",
        KimiToolCall {
            tool_name: "Shell".to_string(),
            kind: ToolCallKind::InteractiveShellMode {
                command: "rm -rf /tmp/demo".to_string(),
            },
        },
    )
    .await
    .expect("non-coverage paths should not hard-fail");

    match proof {
        ManagedExecProof::NonCoverage(note) => {
            assert!(note.reason.contains("shell mode"));
            assert!(note.reason.contains("non-coverage"));
            assert!(note.reason.contains("Ctrl-X"));
        }
        other => panic!("expected non-coverage note, got {other:?}"),
    }
}

#[tokio::test]
async fn daemon_version_mismatch_fails_managed_mode() {
    let temp = tempdir().expect("tempdir");
    let socket_path = temp.path().join("kimi.sock");
    let journal_path = temp.path().join("events.journal");
    let server = spawn_rpc_server(DaemonRpcConfig::new(&socket_path, &journal_path))
        .expect("spawn daemon rpc server");

    let stream = UnixStream::connect(&socket_path)
        .await
        .expect("connect daemon socket");
    let mut framed = Framed::new(stream, LengthDelimitedCodec::new());

    let mut request = HandshakeRequest::new("execmanager-host-kimi");
    request.protocol_version = PROTOCOL_VERSION + 1;
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
        .expect("handshake response frame")
        .expect("handshake response bytes");
    let response: DaemonResponseEnvelope =
        serde_json::from_slice(&frame).expect("decode handshake response");

    match response {
        DaemonResponseEnvelope::Handshake(HandshakeResponse::Rejected(rejected)) => {
            assert_eq!(
                rejected.reason,
                HandshakeRejectReason::IncompatibleProtocolVersion {
                    expected: PROTOCOL_VERSION,
                    actual: PROTOCOL_VERSION + 1,
                }
            );
        }
        other => panic!("expected version rejection, got {other:?}"),
    }

    let error = route_tool_call(
        temp.path().join("missing.sock"),
        KimiToolCall {
            tool_name: "Shell".to_string(),
            kind: ToolCallKind::AgentIssuedShell(ShellToolCall {
                command: "echo mismatch".to_string(),
            }),
        },
    )
    .await
    .expect_err("missing daemon socket must still fail managed mode");
    assert!(matches!(error, IngressError::DaemonUnavailable(_)));

    server.shutdown().await.expect("server shutdown");
}

use execmanager_contracts::{
    evaluate_handshake, CapabilityFlag, CleanupRequest, DaemonAuthResult, DaemonRequestEnvelope,
    DaemonResponseEnvelope, DegradedCapability, DegradedReason, ExecutionId, HandshakeRejectReason,
    HandshakeRequest, HandshakeResponse, LaunchAdmission, LaunchRequest, LaunchResponse,
    PeerIdentity, ProjectionState, RedactionMarker, SnapshotRequest, ViewerRequest,
};
use serde_json::{json, Value};

#[test]
fn compatible_handshake_succeeds() {
    let request = HandshakeRequest::new("execmanager-host-kimi");

    let response = evaluate_handshake(
        &request,
        DaemonAuthResult::AuthenticatedSameUser {
            peer: PeerIdentity {
                user_id: 1000,
                process_id: Some(4242),
                username: Some("kimi".to_string()),
            },
        },
        vec![CapabilityFlag::ManagedExec, CapabilityFlag::ViewerAttach],
        vec![DegradedCapability {
            capability: CapabilityFlag::ResourceGovernance,
            reason: DegradedReason::ObservationOnly,
        }],
    );

    match response {
        DaemonResponseEnvelope::Handshake(HandshakeResponse::Accepted(accepted)) => {
            assert_eq!(
                accepted.protocol_version,
                HandshakeRequest::protocol_version()
            );
            assert_eq!(
                accepted.auth,
                DaemonAuthResult::AuthenticatedSameUser {
                    peer: PeerIdentity {
                        user_id: 1000,
                        process_id: Some(4242),
                        username: Some("kimi".to_string()),
                    },
                }
            );
            assert_eq!(
                accepted.capabilities,
                vec![CapabilityFlag::ManagedExec, CapabilityFlag::ViewerAttach]
            );
            assert_eq!(
                accepted.degraded_capabilities,
                vec![DegradedCapability {
                    capability: CapabilityFlag::ResourceGovernance,
                    reason: DegradedReason::ObservationOnly,
                }]
            );
        }
        other => panic!("expected accepted handshake, got {other:?}"),
    }
}

#[test]
fn incompatible_version_fails_closed() {
    let mut request = HandshakeRequest::new("execmanager-host-kimi");
    request.protocol_version += 1;

    let response = evaluate_handshake(
        &request,
        DaemonAuthResult::AuthenticatedSameUser {
            peer: PeerIdentity {
                user_id: 1000,
                process_id: Some(5000),
                username: Some("kimi".to_string()),
            },
        },
        vec![CapabilityFlag::ManagedExec],
        vec![],
    );

    match response {
        DaemonResponseEnvelope::Handshake(HandshakeResponse::Rejected(rejected)) => {
            assert_eq!(
                rejected.reason,
                HandshakeRejectReason::IncompatibleProtocolVersion {
                    expected: HandshakeRequest::protocol_version(),
                    actual: HandshakeRequest::protocol_version() + 1,
                }
            );
        }
        other => panic!("expected rejected handshake, got {other:?}"),
    }
}

#[test]
fn contract_json_shapes_are_locked() {
    let launch = DaemonRequestEnvelope::Launch(LaunchRequest {
        tool_name: "Shell".to_string(),
        command: "echo managed".to_string(),
        working_dir: Some("/workspace/demo".to_string()),
        source: Some("kimi:shell".to_string()),
    });
    assert_json(
        serde_json::to_value(&launch).expect("serialize launch request"),
        json!({
            "type": "launch",
            "tool_name": "Shell",
            "command": "echo managed",
            "working_dir": "/workspace/demo",
            "source": "kimi:shell"
        }),
    );

    let launch_response = DaemonResponseEnvelope::Launch(LaunchResponse {
        admission: LaunchAdmission::Admitted,
        exec_id: ExecutionId::new("exec-123"),
    });
    assert_json(
        serde_json::to_value(&launch_response).expect("serialize launch response"),
        json!({
            "type": "launch",
            "admission": "admitted",
            "exec_id": "exec-123"
        }),
    );

    let snapshot = DaemonRequestEnvelope::Snapshot(SnapshotRequest {
        exec_id: ExecutionId::new("exec-123"),
    });
    assert_json(
        serde_json::to_value(&snapshot).expect("serialize snapshot request"),
        json!({
            "type": "snapshot",
            "exec_id": "exec-123"
        }),
    );

    let viewer = DaemonRequestEnvelope::Viewer(ViewerRequest {
        exec_id: ExecutionId::new("exec-123"),
    });
    assert_json(
        serde_json::to_value(&viewer).expect("serialize viewer request"),
        json!({
            "type": "viewer",
            "exec_id": "exec-123"
        }),
    );

    let cleanup = DaemonRequestEnvelope::Cleanup(CleanupRequest {
        exec_id: ExecutionId::new("exec-123"),
    });
    assert_json(
        serde_json::to_value(&cleanup).expect("serialize cleanup request"),
        json!({
            "type": "cleanup",
            "exec_id": "exec-123"
        }),
    );

    let projection_state =
        serde_json::to_value(ProjectionState::ShortTask).expect("serialize projection state");
    assert_json(projection_state, json!("short_task"));

    let redaction_marker =
        serde_json::to_value(RedactionMarker::Redacted).expect("serialize redaction marker");
    assert_json(redaction_marker, json!("redacted"));
}

fn assert_json(actual: Value, expected: Value) {
    assert_eq!(actual, expected);
}

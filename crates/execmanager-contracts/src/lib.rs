//! Shared contracts for execmanager components.

use serde::{Deserialize, Serialize};

pub const PROTOCOL_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ExecutionId(String);

impl ExecutionId {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum DaemonRequestEnvelope {
    Handshake(HandshakeRequest),
    Launch(LaunchRequest),
    Snapshot(SnapshotRequest),
    Cleanup(CleanupRequest),
    Viewer(ViewerRequest),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum DaemonResponseEnvelope {
    Handshake(HandshakeResponse),
    Launch(LaunchResponse),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HandshakeRequest {
    pub protocol_version: u32,
    pub client_name: String,
}

impl HandshakeRequest {
    pub fn new(client_name: impl Into<String>) -> Self {
        Self {
            protocol_version: PROTOCOL_VERSION,
            client_name: client_name.into(),
        }
    }

    pub fn protocol_version() -> u32 {
        PROTOCOL_VERSION
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HandshakeResponse {
    Accepted(HandshakeAccepted),
    Rejected(HandshakeRejected),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HandshakeAccepted {
    pub protocol_version: u32,
    pub auth: DaemonAuthResult,
    pub capabilities: Vec<CapabilityFlag>,
    pub degraded_capabilities: Vec<DegradedCapability>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HandshakeRejected {
    pub reason: HandshakeRejectReason,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HandshakeRejectReason {
    IncompatibleProtocolVersion { expected: u32, actual: u32 },
    UnauthenticatedPeer { reason: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DaemonAuthResult {
    AuthenticatedSameUser { peer: PeerIdentity },
    Unauthenticated { reason: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PeerIdentity {
    pub user_id: u32,
    pub process_id: Option<u32>,
    pub username: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityFlag {
    ManagedExec,
    ResourceGovernance,
    ServiceDiscovery,
    SnapshotRead,
    CleanupManagedTree,
    ViewerAttach,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DegradedCapability {
    pub capability: CapabilityFlag,
    pub reason: DegradedReason,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DegradedReason {
    UnsupportedPlatform,
    ObservationOnly,
    Unavailable,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LaunchRequest {
    pub tool_name: String,
    pub command: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub working_dir: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LaunchResponse {
    pub admission: LaunchAdmission,
    pub exec_id: ExecutionId,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LaunchAdmission {
    Admitted,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SnapshotRequest {
    pub exec_id: ExecutionId,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CleanupRequest {
    pub exec_id: ExecutionId,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ViewerRequest {
    pub exec_id: ExecutionId,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProjectionState {
    Managed,
    Service,
    ShortTask,
    Missing,
    Orphaned,
    Escaped,
    Detached,
    Exited,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RedactionMarker {
    Plaintext,
    Redacted,
    Omitted,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RetentionClass {
    MetadataLongLived,
    BlobEphemeral,
    BlobRetained,
}

pub fn evaluate_handshake(
    request: &HandshakeRequest,
    auth: DaemonAuthResult,
    capabilities: Vec<CapabilityFlag>,
    degraded_capabilities: Vec<DegradedCapability>,
) -> DaemonResponseEnvelope {
    let response = if request.protocol_version != PROTOCOL_VERSION {
        HandshakeResponse::Rejected(HandshakeRejected {
            reason: HandshakeRejectReason::IncompatibleProtocolVersion {
                expected: PROTOCOL_VERSION,
                actual: request.protocol_version,
            },
        })
    } else if let DaemonAuthResult::AuthenticatedSameUser { .. } = &auth {
        HandshakeResponse::Accepted(HandshakeAccepted {
            protocol_version: PROTOCOL_VERSION,
            auth,
            capabilities,
            degraded_capabilities,
        })
    } else {
        let reason = match auth {
            DaemonAuthResult::AuthenticatedSameUser { .. } => unreachable!(),
            DaemonAuthResult::Unauthenticated { reason } => reason,
        };
        HandshakeResponse::Rejected(HandshakeRejected {
            reason: HandshakeRejectReason::UnauthenticatedPeer { reason },
        })
    };

    DaemonResponseEnvelope::Handshake(response)
}

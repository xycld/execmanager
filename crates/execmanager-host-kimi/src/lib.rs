use std::fmt;
use std::path::Path;

use execmanager_contracts::{
    DaemonAuthResult, DaemonRequestEnvelope, DaemonResponseEnvelope, HandshakeRejectReason,
    HandshakeRequest, HandshakeResponse, LaunchRequest, LaunchResponse, PROTOCOL_VERSION,
};
use futures::{SinkExt, StreamExt};
use tokio::net::UnixStream;
use tokio_util::codec::{Framed, LengthDelimitedCodec};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KimiToolCall {
    pub tool_name: String,
    pub kind: ToolCallKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolCallKind {
    AgentIssuedShell(ShellToolCall),
    InteractiveShellMode { command: String },
    Unsupported { detail: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShellToolCall {
    pub command: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ManagedExecProof {
    Managed(ManagedLaunch),
    NonCoverage(NonCoverageNote),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManagedLaunch {
    pub exec_id: String,
    pub command: String,
    pub pre_spawn: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NonCoverageNote {
    pub reason: String,
}

#[derive(Debug)]
pub enum IngressError {
    DaemonUnavailable(std::io::Error),
    Io(std::io::Error),
    Serialize(serde_json::Error),
    Decode(serde_json::Error),
    MissingDaemonResponse,
    UnexpectedDaemonResponse(&'static str),
    DaemonProtocolVersionMismatch { expected: u32, actual: u32 },
    DaemonUnauthenticated { reason: String },
}

impl fmt::Display for IngressError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DaemonUnavailable(error) => {
                write!(f, "managed mode failed: daemon unavailable: {error}")
            }
            Self::Io(error) => write!(f, "managed mode failed during daemon IO: {error}"),
            Self::Serialize(error) => write!(f, "managed mode failed to encode request: {error}"),
            Self::Decode(error) => write!(f, "managed mode failed to decode response: {error}"),
            Self::MissingDaemonResponse => {
                write!(f, "managed mode failed: daemon closed without response")
            }
            Self::UnexpectedDaemonResponse(context) => {
                write!(f, "managed mode failed: daemon sent unexpected response during {context}")
            }
            Self::DaemonProtocolVersionMismatch { expected, actual } => write!(
                f,
                "managed mode failed: daemon protocol version mismatch (expected {expected}, got {actual})"
            ),
            Self::DaemonUnauthenticated { reason } => {
                write!(f, "managed mode failed: daemon rejected peer authentication: {reason}")
            }
        }
    }
}

impl std::error::Error for IngressError {}

pub async fn route_tool_call(
    socket_path: impl AsRef<Path>,
    tool_call: KimiToolCall,
) -> Result<ManagedExecProof, IngressError> {
    match tool_call.kind {
        ToolCallKind::AgentIssuedShell(shell) if tool_call.tool_name == "Shell" => {
            route_supported_shell(socket_path.as_ref(), &tool_call.tool_name, shell).await
        }
        ToolCallKind::InteractiveShellMode { .. } => Ok(ManagedExecProof::NonCoverage(
            NonCoverageNote {
                reason: "Explicit non-coverage: Kimi Ctrl-X shell mode bypasses host-routable tool ingress; this proof covers only agent-issued Shell tool calls.".to_string(),
            },
        )),
        ToolCallKind::AgentIssuedShell(_) | ToolCallKind::Unsupported { .. } => {
            Ok(ManagedExecProof::NonCoverage(NonCoverageNote {
                reason: "Explicit non-coverage: only host-routable Kimi Shell tool calls are proven in Task 1; all other ingress classes remain outside this proof boundary.".to_string(),
            }))
        }
    }
}

async fn route_supported_shell(
    socket_path: &Path,
    tool_name: &str,
    shell: ShellToolCall,
) -> Result<ManagedExecProof, IngressError> {
    let stream = UnixStream::connect(socket_path)
        .await
        .map_err(IngressError::DaemonUnavailable)?;
    let mut framed = Framed::new(stream, LengthDelimitedCodec::new());

    let handshake =
        DaemonRequestEnvelope::Handshake(HandshakeRequest::new("execmanager-host-kimi"));
    send_request(&mut framed, &handshake).await?;

    let handshake_response = read_response(&mut framed).await?;
    match handshake_response {
        DaemonResponseEnvelope::Handshake(HandshakeResponse::Accepted(accepted)) => {
            if accepted.protocol_version != PROTOCOL_VERSION {
                return Err(IngressError::DaemonProtocolVersionMismatch {
                    expected: PROTOCOL_VERSION,
                    actual: accepted.protocol_version,
                });
            }

            if let DaemonAuthResult::Unauthenticated { reason } = accepted.auth {
                return Err(IngressError::DaemonUnauthenticated { reason });
            }
        }
        DaemonResponseEnvelope::Handshake(HandshakeResponse::Rejected(rejected)) => {
            return match rejected.reason {
                HandshakeRejectReason::IncompatibleProtocolVersion { expected, actual } => {
                    Err(IngressError::DaemonProtocolVersionMismatch { expected, actual })
                }
                HandshakeRejectReason::UnauthenticatedPeer { reason } => {
                    Err(IngressError::DaemonUnauthenticated { reason })
                }
            };
        }
        _ => return Err(IngressError::UnexpectedDaemonResponse("handshake")),
    }

    let request = DaemonRequestEnvelope::Launch(LaunchRequest {
        tool_name: tool_name.to_string(),
        command: shell.command.clone(),
        working_dir: std::env::current_dir()
            .ok()
            .map(|path| path.display().to_string()),
        source: Some(format!("kimi:{}", tool_name.to_lowercase())),
    });
    send_request(&mut framed, &request).await?;

    let response = read_response(&mut framed).await?;
    let launch = match response {
        DaemonResponseEnvelope::Launch(launch) => launch,
        _ => return Err(IngressError::UnexpectedDaemonResponse("launch")),
    };

    Ok(ManagedExecProof::Managed(ManagedLaunch {
        exec_id: execution_id_to_string(&launch),
        command: shell.command,
        pre_spawn: true,
    }))
}

async fn send_request(
    framed: &mut Framed<UnixStream, LengthDelimitedCodec>,
    request: &DaemonRequestEnvelope,
) -> Result<(), IngressError> {
    let payload = serde_json::to_vec(request).map_err(IngressError::Serialize)?;
    framed.send(payload.into()).await.map_err(IngressError::Io)
}

async fn read_response(
    framed: &mut Framed<UnixStream, LengthDelimitedCodec>,
) -> Result<DaemonResponseEnvelope, IngressError> {
    let response_frame = framed
        .next()
        .await
        .transpose()
        .map_err(IngressError::Io)?
        .ok_or(IngressError::MissingDaemonResponse)?;

    serde_json::from_slice(&response_frame).map_err(IngressError::Decode)
}

fn execution_id_to_string(response: &LaunchResponse) -> String {
    response.exec_id.as_str().to_string()
}

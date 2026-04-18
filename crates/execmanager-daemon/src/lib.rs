//! Durable journal, replay, and managed execution surfaces for execmanager daemon.

use std::collections::{HashMap, HashSet};
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use execmanager_contracts::{
    CapabilityFlag, DaemonAuthResult, DaemonRequestEnvelope, DaemonResponseEnvelope, ExecutionId,
    HandshakeRequest, HandshakeResponse, LaunchAdmission, LaunchRequest, LaunchResponse,
    PeerIdentity, ProjectionState, RedactionMarker, RetentionClass, evaluate_handshake,
};
use execmanager_platform::{
    GovernanceCoordinator, GovernanceRequest, GovernanceSnapshot, ResourceProfile,
};
use futures::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::oneshot;
use tokio::task::JoinHandle;
use tokio_util::codec::{Framed, LengthDelimitedCodec};

#[cfg(unix)]
use std::os::unix::fs::MetadataExt;
#[cfg(unix)]
use std::os::unix::process::CommandExt;

const RECORD_MAGIC: [u8; 4] = *b"EXMJ";
const RECORD_VERSION: u8 = 1;
const HEADER_LEN: usize = 13;
const RM_POLICY_NAME: &str = "rm_safety_adapter";
const OBSERVED_SERVICE_NAME: &str = "observed_service";
const OUTPUT_MEMORY_CAPTURE_LIMIT: usize = 256 * 1024;
const OUTPUT_SPOOL_BUFFER_SIZE: usize = 16 * 1024;
const HISTORY_SNAPSHOT_SCHEMA_VERSION: u32 = 1;
const DEFAULT_RPC_RUNTIME_OBSERVATION_TIMEOUT: Duration = Duration::from_secs(3);

static RPC_EXECUTION_COUNTER: AtomicU64 = AtomicU64::new(1);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DaemonRpcConfig {
    socket_path: PathBuf,
    journal_path: PathBuf,
    runtime_observation_timeout: Duration,
}

impl DaemonRpcConfig {
    pub fn new(socket_path: impl AsRef<Path>, journal_path: impl AsRef<Path>) -> Self {
        Self {
            socket_path: socket_path.as_ref().to_path_buf(),
            journal_path: journal_path.as_ref().to_path_buf(),
            runtime_observation_timeout: DEFAULT_RPC_RUNTIME_OBSERVATION_TIMEOUT,
        }
    }

    pub fn with_runtime_observation_timeout(mut self, timeout: Duration) -> Self {
        self.runtime_observation_timeout = timeout;
        self
    }
}

#[derive(Debug)]
pub struct DaemonRpcServer {
    socket_path: PathBuf,
    shutdown_tx: Option<oneshot::Sender<()>>,
    task: JoinHandle<io::Result<()>>,
}

impl DaemonRpcServer {
    pub async fn shutdown(mut self) -> io::Result<()> {
        if let Some(shutdown_tx) = self.shutdown_tx.take() {
            let _ = shutdown_tx.send(());
        }

        let result = self
            .task
            .await
            .map_err(|error| io::Error::other(error.to_string()))?;

        match fs::remove_file(&self.socket_path) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }

        result
    }
}

pub fn spawn_rpc_server(config: DaemonRpcConfig) -> io::Result<DaemonRpcServer> {
    match fs::remove_file(&config.socket_path) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }

    let listener = UnixListener::bind(&config.socket_path)?;
    let socket_path = config.socket_path.clone();
    let (shutdown_tx, shutdown_rx) = oneshot::channel();
    let task = tokio::spawn(run_rpc_server(listener, config, shutdown_rx));

    Ok(DaemonRpcServer {
        socket_path,
        shutdown_tx: Some(shutdown_tx),
        task,
    })
}

pub async fn probe_rpc_readiness(socket_path: impl AsRef<Path>) -> io::Result<()> {
    let stream = UnixStream::connect(socket_path.as_ref()).await?;
    let mut framed = Framed::new(stream, LengthDelimitedCodec::new());
    let handshake = DaemonRequestEnvelope::Handshake(HandshakeRequest::new("execmanager-cli"));
    let payload = serde_json::to_vec(&handshake)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error.to_string()))?;
    framed.send(payload.into()).await?;

    let frame = framed.next().await.transpose()?.ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::UnexpectedEof,
            "daemon closed without a readiness response",
        )
    })?;
    let response: DaemonResponseEnvelope = serde_json::from_slice(&frame)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error.to_string()))?;

    match response {
        DaemonResponseEnvelope::Handshake(HandshakeResponse::Accepted(_)) => Ok(()),
        DaemonResponseEnvelope::Handshake(HandshakeResponse::Rejected(rejected)) => {
            Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                format!("daemon handshake rejected: {:?}", rejected.reason),
            ))
        }
        other => Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("unexpected daemon readiness response: {other:?}"),
        )),
    }
}

async fn run_rpc_server(
    listener: UnixListener,
    config: DaemonRpcConfig,
    mut shutdown_rx: oneshot::Receiver<()>,
) -> io::Result<()> {
    loop {
        tokio::select! {
            _ = &mut shutdown_rx => return Ok(()),
            accepted = listener.accept() => {
                let (stream, _) = accepted?;
                handle_rpc_connection(stream, &config).await?;
            }
        }
    }
}

async fn handle_rpc_connection(stream: UnixStream, config: &DaemonRpcConfig) -> io::Result<()> {
    let auth = authenticate_peer(&stream)?;
    let mut framed = Framed::new(stream, LengthDelimitedCodec::new());

    let handshake_request = match read_daemon_request(&mut framed).await? {
        DaemonRequestEnvelope::Handshake(request) => request,
        other => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("expected handshake request, got {other:?}"),
            ));
        }
    };

    let handshake_response = evaluate_handshake(
        &handshake_request,
        auth,
        vec![
            CapabilityFlag::ManagedExec,
            CapabilityFlag::ServiceDiscovery,
            CapabilityFlag::ViewerAttach,
        ],
        vec![],
    );
    send_daemon_response(&mut framed, &handshake_response).await?;

    if !matches!(
        handshake_response,
        DaemonResponseEnvelope::Handshake(HandshakeResponse::Accepted(_))
    ) {
        return Ok(());
    }

    let launch_request = match read_daemon_request(&mut framed).await {
        Err(error) if error.kind() == io::ErrorKind::UnexpectedEof => return Ok(()),
        Err(error) => return Err(error),
        Ok(DaemonRequestEnvelope::Launch(request)) => request,
        other => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("expected launch request, got {other:?}"),
            ));
        }
    };

    let exec_id = launch_managed_execution(config, launch_request).await?;
    send_daemon_response(
        &mut framed,
        &DaemonResponseEnvelope::Launch(LaunchResponse {
            admission: LaunchAdmission::Admitted,
            exec_id,
        }),
    )
    .await
}

async fn launch_managed_execution(
    config: &DaemonRpcConfig,
    request: LaunchRequest,
) -> io::Result<ExecutionId> {
    let journal_path = config.journal_path.clone();
    let runtime_observation_timeout = config.runtime_observation_timeout;
    let command = request.command;

    let (exec_id, managed_child) = tokio::task::spawn_blocking(move || {
        let exec_id = next_rpc_execution_id();
        let spec = managed_launch_spec_from_command(exec_id.clone(), &command)?;
        let mut executor = ManagedExecutor::new(&journal_path)?;
        let child = executor
            .launch(spec)
            .map_err(|error| io::Error::other(error.to_string()))?;
        Ok::<_, io::Error>((exec_id, child))
    })
    .await
    .map_err(|error| io::Error::other(error.to_string()))??;

    tokio::task::spawn_blocking(move || {
        let _ = managed_child.observe_runtime_facts(runtime_observation_timeout);
        let _ = managed_child.wait_with_output();
    });

    Ok(exec_id)
}

fn managed_launch_spec_from_command(
    exec_id: ExecutionId,
    command: &str,
) -> io::Result<ManagedLaunchSpec> {
    let argv = split_launch_command(command)?;
    let (program, args) = argv
        .split_first()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "launch command is empty"))?;

    Ok(ManagedLaunchSpec::new(
        exec_id,
        PathBuf::from(program),
        args.to_vec(),
        ExecutionMode::BatchPipes,
    )
    .with_original_command(command))
}

fn split_launch_command(command: &str) -> io::Result<Vec<String>> {
    if command.trim().is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "launch command is empty",
        ));
    }

    if command.contains('"')
        || command.contains('\'')
        || command.contains("$(")
        || command.contains('`')
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "launch command requires unsupported shell parsing",
        ));
    }

    let argv: Vec<String> = command.split_whitespace().map(str::to_string).collect();
    if argv.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "launch command is empty after tokenization",
        ));
    }

    Ok(argv)
}

fn next_rpc_execution_id() -> ExecutionId {
    let counter = RPC_EXECUTION_COUNTER.fetch_add(1, Ordering::Relaxed);
    ExecutionId::new(format!("exec-rpc-{}-{counter}", epoch_millis()))
}

fn authenticate_peer(stream: &UnixStream) -> io::Result<DaemonAuthResult> {
    let credentials = stream.peer_cred()?;
    let peer_uid = credentials.uid();
    let daemon_uid = current_effective_uid();

    if peer_uid != daemon_uid {
        return Ok(DaemonAuthResult::Unauthenticated {
            reason: format!("peer uid {peer_uid} does not match daemon uid {daemon_uid}"),
        });
    }

    Ok(DaemonAuthResult::AuthenticatedSameUser {
        peer: PeerIdentity {
            user_id: peer_uid,
            process_id: credentials.pid().map(|pid| pid as u32),
            username: current_username_for_uid(peer_uid),
        },
    })
}

fn current_effective_uid() -> u32 {
    #[cfg(unix)]
    {
        unsafe { libc::geteuid() as u32 }
    }
}

fn current_username_for_uid(user_id: u32) -> Option<String> {
    #[cfg(unix)]
    {
        fs::read_to_string("/etc/passwd").ok().and_then(|contents| {
            contents.lines().find_map(|line| {
                let mut fields = line.split(':');
                let name = fields.next()?;
                let _password = fields.next()?;
                let uid = fields.next()?.parse::<u32>().ok()?;
                (uid == user_id).then(|| name.to_string())
            })
        })
    }
}

async fn read_daemon_request(
    framed: &mut Framed<UnixStream, LengthDelimitedCodec>,
) -> io::Result<DaemonRequestEnvelope> {
    let frame = framed.next().await.transpose()?.ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::UnexpectedEof,
            "daemon client closed without sending a request",
        )
    })?;

    serde_json::from_slice(&frame)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error.to_string()))
}

async fn send_daemon_response(
    framed: &mut Framed<UnixStream, LengthDelimitedCodec>,
    response: &DaemonResponseEnvelope,
) -> io::Result<()> {
    let payload = serde_json::to_vec(response)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error.to_string()))?;
    framed.send(payload.into()).await
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionMode {
    BatchPipes,
    InteractivePty,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LifecycleStage {
    Requested,
    Admitted,
    Spawned,
    Exited,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeOwnership {
    pub root_pid: u32,
    pub process_group_id: u32,
    pub session_id: Option<u32>,
    pub start_time_ticks: Option<u64>,
}

impl RuntimeOwnership {
    pub fn cleanup_target(&self) -> Result<ManagedCleanupTarget, OwnershipError> {
        if self.root_pid == 0 {
            return Err(OwnershipError::InsufficientProof {
                exec_id: None,
                reason: "managed root pid is missing".to_string(),
            });
        }

        if self.process_group_id == 0 {
            return Err(OwnershipError::InsufficientProof {
                exec_id: None,
                reason: "managed process group id is missing".to_string(),
            });
        }

        Ok(ManagedCleanupTarget {
            root_pid: self.root_pid,
            process_group_id: self.process_group_id,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManagedCleanupTarget {
    pub root_pid: u32,
    pub process_group_id: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OwnershipError {
    InsufficientProof {
        exec_id: Option<ExecutionId>,
        reason: String,
    },
}

impl std::fmt::Display for OwnershipError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InsufficientProof { exec_id, reason } => match exec_id {
                Some(exec_id) => write!(
                    f,
                    "insufficient ownership proof for {}: {reason}",
                    exec_id.as_str()
                ),
                None => write!(f, "insufficient ownership proof: {reason}"),
            },
        }
    }
}

impl std::error::Error for OwnershipError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManagedLaunchSpec {
    pub exec_id: ExecutionId,
    pub program: std::path::PathBuf,
    pub args: Vec<String>,
    pub environment: Vec<(String, String)>,
    pub mode: ExecutionMode,
    pub original_command: String,
}

impl ManagedLaunchSpec {
    pub fn new(
        exec_id: ExecutionId,
        program: impl Into<std::path::PathBuf>,
        args: Vec<String>,
        mode: ExecutionMode,
    ) -> Self {
        let program = program.into();
        let original_command = Self::render_command(&program, &args);
        Self {
            exec_id,
            program,
            args,
            environment: Vec::new(),
            mode,
            original_command,
        }
    }

    pub fn with_env(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.environment.push((name.into(), value.into()));
        self
    }

    pub fn with_original_command(mut self, original_command: impl Into<String>) -> Self {
        self.original_command = original_command.into();
        self
    }

    fn rendered_command(&self) -> String {
        Self::render_command(&self.program, &self.args)
    }

    fn render_command(program: &Path, args: &[String]) -> String {
        let mut rendered = program.display().to_string();
        for arg in args {
            rendered.push(' ');
            rendered.push_str(arg);
        }
        rendered
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum LaunchPolicyOutcome {
    AllowedAsRequested { policy: String, reason: String },
    Rewritten { policy: String, reason: String },
    Denied { policy: String, reason: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ManagedExecError {
    UnsupportedExecutionMode {
        requested: ExecutionMode,
        reason: String,
    },
    PolicyDenied {
        policy: String,
        reason: String,
    },
    Io(String),
    Journal(AppendError),
}

impl From<io::Error> for ManagedExecError {
    fn from(value: io::Error) -> Self {
        Self::Io(value.to_string())
    }
}

impl From<AppendError> for ManagedExecError {
    fn from(value: AppendError) -> Self {
        Self::Journal(value)
    }
}

impl std::fmt::Display for ManagedExecError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnsupportedExecutionMode { requested, reason } => {
                write!(f, "unsupported execution mode {requested:?}: {reason}")
            }
            Self::PolicyDenied { policy, reason } => {
                write!(f, "launch denied by {policy}: {reason}")
            }
            Self::Io(message) => write!(f, "managed execution IO failed: {message}"),
            Self::Journal(error) => write!(f, "managed execution journal failed: {error}"),
        }
    }
}

impl std::error::Error for ManagedExecError {}

#[derive(Debug)]
pub enum ManagedCleanupError {
    Ownership(OwnershipError),
    Io(String),
    Journal(AppendError),
    Timeout { process_group_id: u32 },
}

impl From<OwnershipError> for ManagedCleanupError {
    fn from(value: OwnershipError) -> Self {
        Self::Ownership(value)
    }
}

impl From<io::Error> for ManagedCleanupError {
    fn from(value: io::Error) -> Self {
        Self::Io(value.to_string())
    }
}

impl From<AppendError> for ManagedCleanupError {
    fn from(value: AppendError) -> Self {
        Self::Journal(value)
    }
}

impl std::fmt::Display for ManagedCleanupError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Ownership(error) => write!(f, "cleanup rejected: {error}"),
            Self::Io(message) => write!(f, "cleanup IO failed: {message}"),
            Self::Journal(error) => write!(f, "cleanup journal failed: {error}"),
            Self::Timeout { process_group_id } => {
                write!(
                    f,
                    "cleanup timed out for managed process group {process_group_id}"
                )
            }
        }
    }
}

impl std::error::Error for ManagedCleanupError {}

#[derive(Debug)]
enum CaptureError {
    Io(io::Error),
    Journal(AppendError),
}

impl From<io::Error> for CaptureError {
    fn from(value: io::Error) -> Self {
        Self::Io(value)
    }
}

impl From<AppendError> for CaptureError {
    fn from(value: AppendError) -> Self {
        Self::Journal(value)
    }
}

impl From<CaptureError> for ManagedExecError {
    fn from(value: CaptureError) -> Self {
        match value {
            CaptureError::Io(error) => ManagedExecError::Io(error.to_string()),
            CaptureError::Journal(error) => ManagedExecError::Journal(error),
        }
    }
}

impl From<CaptureError> for ManagedCleanupError {
    fn from(value: CaptureError) -> Self {
        match value {
            CaptureError::Io(error) => ManagedCleanupError::Io(error.to_string()),
            CaptureError::Journal(error) => ManagedCleanupError::Journal(error),
        }
    }
}

#[derive(Debug)]
pub struct ManagedExecutor {
    journal: Arc<Mutex<Journal>>,
    governance: GovernanceCoordinator,
}

impl ManagedExecutor {
    pub fn new(path: impl AsRef<Path>) -> io::Result<Self> {
        Self::new_with_governance(
            path,
            GovernanceCoordinator::for_environment(
                execmanager_platform::GovernanceEnvironment::current(),
            ),
        )
    }

    pub fn new_with_governance(
        path: impl AsRef<Path>,
        governance: GovernanceCoordinator,
    ) -> io::Result<Self> {
        Ok(Self {
            journal: Arc::new(Mutex::new(Journal::open(path)?)),
            governance,
        })
    }

    pub fn launch(&mut self, spec: ManagedLaunchSpec) -> Result<ManagedChild, ManagedExecError> {
        if matches!(spec.mode, ExecutionMode::InteractivePty) {
            return Err(ManagedExecError::UnsupportedExecutionMode {
                requested: ExecutionMode::InteractivePty,
                reason: "PTY-backed interactive execution is not supported yet".to_string(),
            });
        }

        let env_snapshot = build_environment_snapshot(&spec.exec_id, &spec.environment);
        let original_command = spec.original_command.clone();
        let persisted_original_command = redact_command_string_for_persistence(&original_command);
        self.append_event(JournalEvent::LaunchRequested {
            exec_id: spec.exec_id.clone(),
            original_command: persisted_original_command.clone(),
            mode: spec.mode.clone(),
        })?;

        let evaluated = evaluate_launch_policy(spec)?;
        let persisted_rewritten_command = evaluated
            .rewritten_command
            .clone()
            .map(|command| redact_command_string_for_persistence(&command));
        self.append_event(JournalEvent::LaunchPolicyEvaluated {
            exec_id: evaluated.effective_spec.exec_id.clone(),
            rewritten_command: persisted_rewritten_command.clone(),
            outcome: evaluated.outcome.clone(),
        })?;

        if let LaunchPolicyOutcome::Denied { policy, reason } = evaluated.outcome {
            return Err(ManagedExecError::PolicyDenied { policy, reason });
        }

        let spec = evaluated.effective_spec;
        let command = spec.rendered_command();
        self.append_event(JournalEvent::LaunchAdmitted {
            exec_id: spec.exec_id.clone(),
            mode: spec.mode.clone(),
        })?;

        let mut process = Command::new(&spec.program);
        process.args(&spec.args);
        for (name, value) in &spec.environment {
            process.env(name, value);
        }
        process.stdin(Stdio::null());
        process.stdout(Stdio::piped());
        process.stderr(Stdio::piped());

        #[cfg(unix)]
        unsafe {
            process.pre_exec(|| {
                if libc::setsid() == -1 {
                    Err(io::Error::last_os_error())
                } else {
                    Ok(())
                }
            });
        }

        let mut child = process.spawn()?;
        let root_pid = child.id();
        let ownership = RuntimeOwnership {
            root_pid,
            process_group_id: process_group_id_for(root_pid)?,
            session_id: session_id_for(root_pid),
            start_time_ticks: process_start_time_ticks(root_pid)?,
        };
        let governance_snapshot = self.capture_governance(spec.exec_id.as_str(), root_pid)?;

        self.append_event(JournalEvent::ProcessSpawned {
            exec_id: spec.exec_id.clone(),
            state: ProjectionState::Managed,
            mode: spec.mode.clone(),
            ownership: ownership.clone(),
            stdout: None,
            stderr: None,
        })?;
        self.append_event(JournalEvent::ResourceGovernanceRecorded {
            exec_id: spec.exec_id.clone(),
            snapshot: governance_snapshot,
        })?;
        let spool_root = self.spool_root_for(&spec.exec_id)?;
        fs::create_dir_all(&spool_root)?;
        let stdout_capture = start_spooled_capture(
            child.stdout.take(),
            spool_root.join("stdout.log"),
            format!("{}-stdout", spec.exec_id.as_str()),
        );
        let stderr_capture = start_spooled_capture(
            child.stderr.take(),
            spool_root.join("stderr.log"),
            format!("{}-stderr", spec.exec_id.as_str()),
        );

        Ok(ManagedChild {
            exec_id: spec.exec_id,
            command,
            original_command,
            persisted_original_command,
            persisted_rewritten_command,
            mode: spec.mode,
            ownership,
            child,
            journal: Arc::clone(&self.journal),
            env_snapshot,
            stdout_capture,
            stderr_capture,
        })
    }

    fn append_event(&self, event: JournalEvent) -> Result<(), ManagedExecError> {
        let mut journal = self
            .journal
            .lock()
            .map_err(|_| io::Error::other("managed executor journal mutex poisoned"))?;
        journal.append(&event)?;
        Ok(())
    }

    fn capture_governance(
        &self,
        exec_id: &str,
        pid: u32,
    ) -> Result<GovernanceSnapshot, ManagedExecError> {
        let mut plan = self.governance.prepare(GovernanceRequest::new(
            exec_id.to_string(),
            ResourceProfile::default(),
        ));
        plan.apply_to_pid(pid);
        Ok(plan.capture().clone())
    }

    fn spool_root_for(&self, exec_id: &ExecutionId) -> Result<PathBuf, ManagedExecError> {
        let journal = self
            .journal
            .lock()
            .map_err(|_| io::Error::other("managed executor journal mutex poisoned"))?;
        Ok(journal
            .path()
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join("blobs")
            .join(exec_id.as_str()))
    }

    pub fn override_service_classification(
        &self,
        exec_id: &ExecutionId,
        state: ProjectionState,
    ) -> Result<(), ManagedExecError> {
        self.append_event(JournalEvent::ServiceOverrideApplied {
            exec_id: exec_id.clone(),
            state,
        })
    }
}

#[derive(Debug)]
pub struct ManagedChild {
    exec_id: ExecutionId,
    command: String,
    original_command: String,
    persisted_original_command: String,
    persisted_rewritten_command: Option<String>,
    mode: ExecutionMode,
    ownership: RuntimeOwnership,
    child: Child,
    journal: Arc<Mutex<Journal>>,
    env_snapshot: EnvironmentSnapshotRecord,
    stdout_capture: BackgroundCapture,
    stderr_capture: BackgroundCapture,
}

impl ManagedChild {
    pub fn ownership(&self) -> &RuntimeOwnership {
        &self.ownership
    }

    pub fn observe_runtime_facts(
        &self,
        timeout: Duration,
    ) -> Result<RuntimeObservation, ManagedExecError> {
        let deadline = Instant::now() + timeout;
        let mut last_error = None;

        loop {
            match observed_listeners_for_pid(self.ownership.root_pid) {
                Ok(listeners) if !listeners.is_empty() => {
                    let service = ServiceView {
                        name: OBSERVED_SERVICE_NAME.to_string(),
                        state: ProjectionState::Service,
                        port_ids: listeners
                            .iter()
                            .map(|listener| listener.port_id.clone())
                            .collect(),
                    };

                    let mut journal = self
                        .journal
                        .lock()
                        .map_err(|_| io::Error::other("managed child journal mutex poisoned"))?;

                    for listener in &listeners {
                        journal.append(&JournalEvent::PortObserved {
                            exec_id: self.exec_id.clone(),
                            port: PortView {
                                port_id: listener.port_id.clone(),
                                port: listener.port,
                                protocol: listener.protocol.clone(),
                                state: ProjectionState::Service,
                            },
                        })?;
                    }

                    journal.append(&JournalEvent::ServiceObserved {
                        exec_id: self.exec_id.clone(),
                        service: service.clone(),
                    })?;

                    return Ok(RuntimeObservation {
                        service_name: service.name,
                        listeners,
                    });
                }
                Ok(_) => {}
                Err(error) => last_error = Some(error),
            }

            if Instant::now() >= deadline {
                break;
            }

            thread::sleep(Duration::from_millis(50));
        }

        let reason = last_error
            .map(|error| error.to_string())
            .unwrap_or_else(|| "no runtime listeners were observed before timeout".to_string());
        Err(ManagedExecError::Io(reason))
    }

    pub fn wait_with_output(mut self) -> Result<Output, ManagedExecError> {
        let status = self.child.wait()?;
        self.finish_with_status(status)
            .map_err(ManagedExecError::from)
    }

    pub fn cleanup(mut self, timeout: Duration) -> Result<Output, ManagedCleanupError> {
        let target = self.ownership.cleanup_target()?;
        signal_process_group(target.process_group_id, libc::SIGTERM)?;

        let deadline = Instant::now() + timeout;
        let status = loop {
            if let Some(status) = self.child.try_wait()? {
                break status;
            }

            if Instant::now() >= deadline {
                signal_process_group(target.process_group_id, libc::SIGKILL)?;
                if let Some(status) = self.child.try_wait()? {
                    break status;
                }
                return Err(ManagedCleanupError::Timeout {
                    process_group_id: target.process_group_id,
                });
            }

            thread::sleep(Duration::from_millis(10));
        };

        self.finish_with_status(status)
            .map_err(ManagedCleanupError::from)
    }

    fn finish_with_status(
        &mut self,
        status: std::process::ExitStatus,
    ) -> Result<Output, CaptureError> {
        let stdout_capture = self.stdout_capture.join()?;
        let stderr_capture = self.stderr_capture.join()?;

        let stdout_blob = stdout_capture.as_retained_artifact(RetentionClass::BlobEphemeral)?;
        let stderr_blob = stderr_capture.as_retained_artifact(RetentionClass::BlobEphemeral)?;
        let manifest = build_history_snapshot_manifest(
            &self.exec_id,
            &self.env_snapshot,
            HistorySnapshotManifestInput {
                persisted_original_command: &self.persisted_original_command,
                persisted_rewritten_command: self.persisted_rewritten_command.clone(),
                observed: SnapshotObservedFacts {
                    final_state: ProjectionState::Exited,
                    exit_code: status.code(),
                    success: status.success(),
                },
                artifacts: SnapshotArtifacts {
                    stdout: stdout_blob.clone(),
                    stderr: stderr_blob.clone(),
                },
            },
        );

        let mut journal = self
            .journal
            .lock()
            .map_err(|_| io::Error::other("managed child journal mutex poisoned"))?;
        journal.append(&JournalEvent::ExecutionStateUpdated {
            exec_id: self.exec_id.clone(),
            state: ProjectionState::Exited,
        })?;
        journal.append(&JournalEvent::HistorySnapshotRecorded {
            exec_id: self.exec_id.clone(),
            env_snapshot: self.env_snapshot.clone(),
            manifest: Box::new(manifest),
        })?;

        Ok(Output {
            status,
            stdout: stdout_capture.into_memory_bytes(),
            stderr: stderr_capture.into_memory_bytes(),
        })
    }

    pub fn command(&self) -> &str {
        &self.command
    }

    pub fn original_command(&self) -> &str {
        &self.original_command
    }

    pub fn mode(&self) -> &ExecutionMode {
        &self.mode
    }
}

#[derive(Debug)]
struct BackgroundCapture {
    handle: Option<thread::JoinHandle<io::Result<SpooledOutput>>>,
}

impl BackgroundCapture {
    fn join(&mut self) -> Result<SpooledOutput, CaptureError> {
        let Some(handle) = self.handle.take() else {
            return Ok(SpooledOutput::empty(String::new(), PathBuf::new()));
        };

        let result = handle
            .join()
            .map_err(|_| io::Error::other("spool reader thread panicked"))?;
        result.map_err(CaptureError::from)
    }
}

#[derive(Debug)]
struct SpooledOutput {
    blob_id: String,
    path: PathBuf,
    sha256: String,
    size_bytes: u64,
    truncated: bool,
    memory: Vec<u8>,
}

impl SpooledOutput {
    fn empty(blob_id: String, path: PathBuf) -> Self {
        Self {
            blob_id,
            path,
            sha256: sha256_hex(&[]),
            size_bytes: 0,
            truncated: false,
            memory: Vec::new(),
        }
    }

    fn as_retained_artifact(
        &self,
        retention: RetentionClass,
    ) -> Result<Option<RetainedArtifact>, CaptureError> {
        if self.size_bytes == 0 {
            return Ok(None);
        }

        Ok(Some(RetainedArtifact {
            blob: BlobReference {
                blob_id: self.blob_id.clone(),
                sha256: self.sha256.clone(),
                size_bytes: self.size_bytes,
                media_type: "text/plain".to_string(),
                storage_path: self.path.display().to_string(),
            },
            retention,
            truncated: self.truncated,
        }))
    }

    fn into_memory_bytes(self) -> Vec<u8> {
        self.memory
    }
}

fn start_spooled_capture(
    stream: Option<impl Read + Send + 'static>,
    path: PathBuf,
    blob_id: String,
) -> BackgroundCapture {
    let handle = thread::spawn(move || spool_stream(stream, path, blob_id));
    BackgroundCapture {
        handle: Some(handle),
    }
}

fn spool_stream(
    stream: Option<impl Read>,
    path: PathBuf,
    blob_id: String,
) -> io::Result<SpooledOutput> {
    let Some(mut stream) = stream else {
        return Ok(SpooledOutput::empty(blob_id, path));
    };

    let mut file = File::create(&path)?;
    let mut digest = Sha256::new();
    let mut memory = Vec::new();
    let mut truncated = false;
    let mut total = 0u64;
    let mut buffer = vec![0u8; OUTPUT_SPOOL_BUFFER_SIZE];

    loop {
        let read = stream.read(&mut buffer)?;
        if read == 0 {
            break;
        }

        let chunk = &buffer[..read];
        file.write_all(chunk)?;
        digest.update(chunk);
        total += read as u64;

        let remaining = OUTPUT_MEMORY_CAPTURE_LIMIT.saturating_sub(memory.len());
        if remaining > 0 {
            let to_copy = remaining.min(read);
            memory.extend_from_slice(&chunk[..to_copy]);
            truncated |= to_copy < read;
        } else {
            truncated = true;
        }
    }

    file.flush()?;
    file.sync_data()?;

    Ok(SpooledOutput {
        blob_id,
        path,
        sha256: hex_digest(digest.finalize()),
        size_bytes: total,
        truncated,
        memory,
    })
}

#[derive(Debug)]
pub struct Journal {
    file: File,
    path: PathBuf,
}

impl Journal {
    pub fn open(path: impl AsRef<Path>) -> io::Result<Self> {
        let path = path.as_ref();
        let file = OpenOptions::new().create(true).append(true).open(path)?;
        Ok(Self {
            file,
            path: path.to_path_buf(),
        })
    }

    pub fn append(&mut self, event: &JournalEvent) -> Result<(), AppendError> {
        let payload = serde_json::to_vec(event)?;
        let checksum = crc32(&payload);
        let payload_len = u32::try_from(payload.len()).map_err(|_| AppendError::PayloadTooLarge)?;

        self.file.write_all(&RECORD_MAGIC)?;
        self.file.write_all(&[RECORD_VERSION])?;
        self.file.write_all(&payload_len.to_le_bytes())?;
        self.file.write_all(&checksum.to_le_bytes())?;
        self.file.write_all(&payload)?;
        self.file.flush()?;
        self.file.sync_data()?;
        Ok(())
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AppendError {
    Io(String),
    Serialize(String),
    PayloadTooLarge,
}

impl From<io::Error> for AppendError {
    fn from(value: io::Error) -> Self {
        Self::Io(value.to_string())
    }
}

impl From<serde_json::Error> for AppendError {
    fn from(value: serde_json::Error) -> Self {
        Self::Serialize(value.to_string())
    }
}

impl std::fmt::Display for AppendError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(message) => write!(f, "journal IO failed: {message}"),
            Self::Serialize(message) => write!(f, "journal serialization failed: {message}"),
            Self::PayloadTooLarge => write!(f, "journal payload exceeded u32 framing limit"),
        }
    }
}

impl std::error::Error for AppendError {}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum JournalEvent {
    LaunchRequested {
        exec_id: ExecutionId,
        original_command: String,
        mode: ExecutionMode,
    },
    LaunchPolicyEvaluated {
        exec_id: ExecutionId,
        rewritten_command: Option<String>,
        outcome: LaunchPolicyOutcome,
    },
    LaunchAdmitted {
        exec_id: ExecutionId,
        mode: ExecutionMode,
    },
    ProcessSpawned {
        exec_id: ExecutionId,
        state: ProjectionState,
        mode: ExecutionMode,
        ownership: RuntimeOwnership,
        stdout: Option<BlobReference>,
        stderr: Option<BlobReference>,
    },
    ExecutionRegistered {
        exec_id: ExecutionId,
        state: ProjectionState,
        command: String,
        stdout: Option<BlobReference>,
        stderr: Option<BlobReference>,
    },
    ExecutionStateUpdated {
        exec_id: ExecutionId,
        state: ProjectionState,
    },
    ServiceObserved {
        exec_id: ExecutionId,
        service: ServiceView,
    },
    ServiceOverrideApplied {
        exec_id: ExecutionId,
        state: ProjectionState,
    },
    PortObserved {
        exec_id: ExecutionId,
        port: PortView,
    },
    GhostStateRecorded {
        exec_id: ExecutionId,
        ghost: GhostView,
    },
    ResourceGovernanceRecorded {
        exec_id: ExecutionId,
        snapshot: GovernanceSnapshot,
    },
    HistorySnapshotRecorded {
        exec_id: ExecutionId,
        env_snapshot: EnvironmentSnapshotRecord,
        manifest: Box<HistorySnapshotManifest>,
    },
}

struct HistorySnapshotManifestInput<'a> {
    persisted_original_command: &'a str,
    persisted_rewritten_command: Option<String>,
    observed: SnapshotObservedFacts,
    artifacts: SnapshotArtifacts,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BlobReference {
    pub blob_id: String,
    pub sha256: String,
    pub size_bytes: u64,
    pub media_type: String,
    pub storage_path: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RetainedArtifact {
    pub blob: BlobReference,
    pub retention: RetentionClass,
    pub truncated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EnvironmentEntryRecord {
    pub name: String,
    pub value: Option<String>,
    pub name_redaction: RedactionMarker,
    pub value_redaction: RedactionMarker,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EnvironmentSnapshotRecord {
    pub record_id: String,
    pub redaction: RedactionMarker,
    pub retention: RetentionClass,
    pub entries: Vec<EnvironmentEntryRecord>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SnapshotExecutionIdentity {
    pub exec_id: ExecutionId,
    pub lineage_root_exec_id: ExecutionId,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SnapshotIntentRecord {
    pub original_command: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SnapshotLaunchRecord {
    pub rewritten_command: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SnapshotObservedFacts {
    pub final_state: ProjectionState,
    pub exit_code: Option<i32>,
    pub success: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SnapshotEnvironmentReference {
    pub record_id: String,
    pub redaction: RedactionMarker,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SnapshotArtifacts {
    pub stdout: Option<RetainedArtifact>,
    pub stderr: Option<RetainedArtifact>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SnapshotHostMetadata {
    pub platform: String,
    pub arch: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HistorySnapshotManifest {
    pub snapshot_id: String,
    pub schema_version: u32,
    pub captured_at_epoch_ms: u64,
    pub execution: SnapshotExecutionIdentity,
    pub intent: SnapshotIntentRecord,
    pub launch: SnapshotLaunchRecord,
    pub observed: SnapshotObservedFacts,
    pub environment: SnapshotEnvironmentReference,
    pub artifacts: SnapshotArtifacts,
    pub host: SnapshotHostMetadata,
    pub retention: RetentionClass,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutionView {
    pub exec_id: ExecutionId,
    pub state: ProjectionState,
    pub observed_state: ProjectionState,
    pub command: String,
    pub original_command: String,
    pub rewritten_command: Option<String>,
    pub policy_outcome: Option<LaunchPolicyOutcome>,
    pub stdout: Option<BlobReference>,
    pub stderr: Option<BlobReference>,
    pub mode: Option<ExecutionMode>,
    pub lifecycle: Vec<LifecycleStage>,
    pub ownership: Option<RuntimeOwnership>,
    pub resource_governance: Option<GovernanceSnapshot>,
    pub service_override: Option<ProjectionState>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ServiceView {
    pub name: String,
    pub state: ProjectionState,
    pub port_ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PortView {
    pub port_id: String,
    pub port: u16,
    pub protocol: String,
    pub state: ProjectionState,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GhostView {
    pub state: ProjectionState,
    pub detail: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ObservedProcess {
    pub root_pid: u32,
    pub process_group_id: u32,
    pub session_id: Option<u32>,
    pub start_time_ticks: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ReconcileSummary {
    pub unknown_processes: Vec<ObservedProcess>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObservedListener {
    pub port_id: String,
    pub port: u16,
    pub protocol: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeObservation {
    pub service_name: String,
    pub listeners: Vec<ObservedListener>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ViewerHandle {
    pub exec_id: ExecutionId,
    pub stdout: Option<BlobReference>,
    pub stderr: Option<BlobReference>,
    pub journal_path: Option<PathBuf>,
}

impl ViewerHandle {
    pub fn from_execution(
        execution: &ExecutionView,
        journal_path: Option<PathBuf>,
    ) -> Result<Self, ViewerHandleError> {
        if execution.ownership.is_none() {
            return Err(ViewerHandleError::Unavailable {
                exec_id: execution.exec_id.clone(),
                reason: "execution has no daemon-owned runtime ownership proof".to_string(),
            });
        }

        Ok(Self {
            exec_id: execution.exec_id.clone(),
            stdout: execution.stdout.clone(),
            stderr: execution.stderr.clone(),
            journal_path,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ViewerHandleError {
    UnknownExecution {
        exec_id: ExecutionId,
    },
    Unavailable {
        exec_id: ExecutionId,
        reason: String,
    },
}

impl std::fmt::Display for ViewerHandleError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnknownExecution { exec_id } => {
                write!(f, "no managed execution found for {}", exec_id.as_str())
            }
            Self::Unavailable { exec_id, reason } => write!(
                f,
                "viewer handle for {} is unavailable: {reason}",
                exec_id.as_str()
            ),
        }
    }
}

impl std::error::Error for ViewerHandleError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeProjection {
    executions: HashMap<String, ExecutionView>,
    services: HashMap<String, HashMap<String, ServiceView>>,
    ports: HashMap<String, HashMap<String, PortView>>,
    ghosts: HashMap<String, GhostView>,
    env_snapshots: HashMap<String, EnvironmentSnapshotRecord>,
    history_manifests: HashMap<String, HistorySnapshotManifest>,
    degraded: Option<ReplayError>,
    journal_path: Option<std::path::PathBuf>,
}

impl RuntimeProjection {
    pub fn replay_from_path(path: impl AsRef<Path>) -> Result<Self, ReplayError> {
        replay_projection(path.as_ref(), false)
    }

    pub fn replay_with_degraded_state(path: impl AsRef<Path>) -> Result<Self, ReplayError> {
        replay_projection(path.as_ref(), true)
    }

    pub fn execution(&self, exec_id: &str) -> Option<&ExecutionView> {
        self.executions.get(exec_id)
    }

    pub fn service(&self, exec_id: &str, name: &str) -> Option<&ServiceView> {
        self.services
            .get(exec_id)
            .and_then(|services| services.get(name))
    }

    pub fn port(&self, exec_id: &str, port_id: &str) -> Option<&PortView> {
        self.ports.get(exec_id).and_then(|ports| ports.get(port_id))
    }

    pub fn ghost(&self, exec_id: &str) -> Option<&GhostView> {
        self.ghosts.get(exec_id)
    }

    pub fn env_snapshot(&self, exec_id: &str) -> Option<&EnvironmentSnapshotRecord> {
        self.env_snapshots.get(exec_id)
    }

    pub fn history_manifest(&self, exec_id: &str) -> Option<&HistorySnapshotManifest> {
        self.history_manifests.get(exec_id)
    }

    pub fn is_degraded(&self) -> bool {
        self.degraded.is_some()
    }

    pub fn degraded_reason(&self) -> Option<&ReplayError> {
        self.degraded.as_ref()
    }

    pub fn journal_path(&self) -> Option<&Path> {
        self.journal_path.as_deref()
    }

    pub fn history(&self) -> Result<Vec<RecordedJournalEvent>, ReplayError> {
        match self.journal_path() {
            Some(path) => read_journal_events(path),
            None => Ok(Vec::new()),
        }
    }

    pub fn viewer_handle(&self, exec_id: &ExecutionId) -> Result<ViewerHandle, ViewerHandleError> {
        let execution = self.execution(exec_id.as_str()).ok_or_else(|| {
            ViewerHandleError::UnknownExecution {
                exec_id: exec_id.clone(),
            }
        })?;
        ViewerHandle::from_execution(execution, self.journal_path.clone())
    }

    pub fn reconcile_with_observed_processes(
        &mut self,
        observed: &[ObservedProcess],
    ) -> ReconcileSummary {
        let mut matched_indices = HashSet::new();
        let exec_ids: Vec<String> = self.executions.keys().cloned().collect();

        for exec_id in exec_ids {
            let Some(execution) = self.executions.get(&exec_id) else {
                continue;
            };

            let ownership = execution.ownership.clone();
            let has_runtime_artifacts = self
                .services
                .get(&exec_id)
                .map(|services| !services.is_empty())
                .unwrap_or(false)
                || self
                    .ports
                    .get(&exec_id)
                    .map(|ports| !ports.is_empty())
                    .unwrap_or(false);
            let exited = execution.state == ProjectionState::Exited
                || execution.observed_state == ProjectionState::Exited
                || execution.lifecycle.contains(&LifecycleStage::Exited);

            let Some(ownership) = ownership else {
                continue;
            };

            if let Some((index, _)) = observed.iter().enumerate().find(|(index, process)| {
                !matched_indices.contains(index) && ownership_matches(&ownership, process)
            }) {
                matched_indices.insert(index);
                self.mark_execution_live(&exec_id);
                continue;
            }

            if exited {
                self.clear_ghost(&exec_id);
                continue;
            }

            if observed
                .iter()
                .any(|process| process.root_pid == ownership.root_pid)
            {
                self.set_uncertain_state(
                    &exec_id,
                    ProjectionState::Escaped,
                    "observed process did not match recorded ownership proof",
                );
                continue;
            }

            if has_runtime_artifacts {
                self.set_uncertain_state(
                    &exec_id,
                    ProjectionState::Detached,
                    "managed root process is gone but runtime artifacts were previously observed",
                );
                continue;
            }

            self.set_uncertain_state(
                &exec_id,
                ProjectionState::Missing,
                "managed root process is not visible in current OS observation",
            );
        }

        ReconcileSummary {
            unknown_processes: observed
                .iter()
                .enumerate()
                .filter_map(|(index, process)| {
                    (!matched_indices.contains(&index)).then_some(process.clone())
                })
                .collect(),
        }
    }

    pub fn cleanup_target(
        &self,
        exec_id: &ExecutionId,
    ) -> Result<ManagedCleanupTarget, OwnershipError> {
        let execution =
            self.execution(exec_id.as_str())
                .ok_or_else(|| OwnershipError::InsufficientProof {
                    exec_id: Some(exec_id.clone()),
                    reason: "execution is not present in runtime projection".to_string(),
                })?;

        if !matches!(
            execution.observed_state,
            ProjectionState::Managed | ProjectionState::Service
        ) {
            return Err(OwnershipError::InsufficientProof {
                exec_id: Some(exec_id.clone()),
                reason: format!(
                    "execution is in uncertain reconciled state {}",
                    projection_state_name(&execution.observed_state)
                ),
            });
        }

        execution
            .ownership
            .as_ref()
            .ok_or_else(|| OwnershipError::InsufficientProof {
                exec_id: Some(exec_id.clone()),
                reason: "execution has no daemon-owned runtime ownership proof".to_string(),
            })?
            .cleanup_target()
            .map_err(|error| match error {
                OwnershipError::InsufficientProof { reason, .. } => {
                    OwnershipError::InsufficientProof {
                        exec_id: Some(exec_id.clone()),
                        reason,
                    }
                }
            })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReplayError {
    Io(String),
    CorruptRecord { offset: u64, kind: CorruptionKind },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordedJournalEvent {
    pub offset: u64,
    pub retention: RetentionClass,
    pub event: JournalEvent,
}

impl std::fmt::Display for ReplayError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(message) => write!(f, "journal replay IO failed: {message}"),
            Self::CorruptRecord { offset, kind } => {
                write!(f, "journal corruption at offset {offset}: {kind}")
            }
        }
    }
}

impl std::error::Error for ReplayError {}

pub fn read_journal_events(
    path: impl AsRef<Path>,
) -> Result<Vec<RecordedJournalEvent>, ReplayError> {
    let bytes = fs::read(path).map_err(|error| ReplayError::Io(error.to_string()))?;
    let mut offset = 0usize;
    let mut records = Vec::new();

    while offset < bytes.len() {
        let record_start = offset as u64;
        if bytes.len() - offset < HEADER_LEN {
            return Err(ReplayError::CorruptRecord {
                offset: record_start,
                kind: CorruptionKind::TruncatedHeader,
            });
        }

        let header = &bytes[offset..offset + HEADER_LEN];
        if header[..4] != RECORD_MAGIC {
            return Err(ReplayError::CorruptRecord {
                offset: record_start,
                kind: CorruptionKind::InvalidMagic,
            });
        }

        let version = header[4];
        if version != RECORD_VERSION {
            return Err(ReplayError::CorruptRecord {
                offset: record_start,
                kind: CorruptionKind::UnsupportedVersion(version),
            });
        }

        let payload_len = u32::from_le_bytes(header[5..9].try_into().expect("u32 header slice"));
        let checksum = u32::from_le_bytes(header[9..13].try_into().expect("u32 header slice"));
        offset += HEADER_LEN;

        let payload_len = payload_len as usize;
        if bytes.len() - offset < payload_len {
            return Err(ReplayError::CorruptRecord {
                offset: record_start,
                kind: CorruptionKind::TruncatedPayload {
                    expected: payload_len,
                    actual: bytes.len() - offset,
                },
            });
        }

        let payload = &bytes[offset..offset + payload_len];
        if crc32(payload) != checksum {
            return Err(ReplayError::CorruptRecord {
                offset: record_start,
                kind: CorruptionKind::ChecksumMismatch,
            });
        }

        let event: JournalEvent =
            serde_json::from_slice(payload).map_err(|error| ReplayError::CorruptRecord {
                offset: record_start,
                kind: CorruptionKind::InvalidJson(error.to_string()),
            })?;
        records.push(RecordedJournalEvent {
            offset: record_start,
            retention: RetentionClass::MetadataLongLived,
            event,
        });
        offset += payload_len;
    }

    Ok(records)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CorruptionKind {
    TruncatedHeader,
    InvalidMagic,
    UnsupportedVersion(u8),
    TruncatedPayload { expected: usize, actual: usize },
    ChecksumMismatch,
    InvalidJson(String),
}

impl std::fmt::Display for CorruptionKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TruncatedHeader => write!(f, "truncated header"),
            Self::InvalidMagic => write!(f, "invalid record magic"),
            Self::UnsupportedVersion(version) => write!(f, "unsupported record version {version}"),
            Self::TruncatedPayload { expected, actual } => {
                write!(
                    f,
                    "truncated payload (expected {expected} bytes, got {actual})"
                )
            }
            Self::ChecksumMismatch => write!(f, "checksum mismatch"),
            Self::InvalidJson(message) => write!(f, "invalid JSON payload: {message}"),
        }
    }
}

fn replay_projection(
    path: &Path,
    degrade_on_error: bool,
) -> Result<RuntimeProjection, ReplayError> {
    let bytes = fs::read(path).map_err(|error| ReplayError::Io(error.to_string()))?;
    let mut projection = RuntimeProjection {
        executions: HashMap::new(),
        services: HashMap::new(),
        ports: HashMap::new(),
        ghosts: HashMap::new(),
        env_snapshots: HashMap::new(),
        history_manifests: HashMap::new(),
        degraded: None,
        journal_path: Some(path.to_path_buf()),
    };
    let mut offset = 0usize;

    while offset < bytes.len() {
        let record_start = offset as u64;
        if bytes.len() - offset < HEADER_LEN {
            let error = ReplayError::CorruptRecord {
                offset: record_start,
                kind: CorruptionKind::TruncatedHeader,
            };
            return finish_replay(projection, error, degrade_on_error);
        }

        let header = &bytes[offset..offset + HEADER_LEN];
        if header[..4] != RECORD_MAGIC {
            let error = ReplayError::CorruptRecord {
                offset: record_start,
                kind: CorruptionKind::InvalidMagic,
            };
            return finish_replay(projection, error, degrade_on_error);
        }

        let version = header[4];
        if version != RECORD_VERSION {
            let error = ReplayError::CorruptRecord {
                offset: record_start,
                kind: CorruptionKind::UnsupportedVersion(version),
            };
            return finish_replay(projection, error, degrade_on_error);
        }

        let payload_len = u32::from_le_bytes(header[5..9].try_into().expect("u32 header slice"));
        let checksum = u32::from_le_bytes(header[9..13].try_into().expect("u32 header slice"));
        offset += HEADER_LEN;

        let payload_len = payload_len as usize;
        if bytes.len() - offset < payload_len {
            let error = ReplayError::CorruptRecord {
                offset: record_start,
                kind: CorruptionKind::TruncatedPayload {
                    expected: payload_len,
                    actual: bytes.len() - offset,
                },
            };
            return finish_replay(projection, error, degrade_on_error);
        }

        let payload = &bytes[offset..offset + payload_len];
        if crc32(payload) != checksum {
            let error = ReplayError::CorruptRecord {
                offset: record_start,
                kind: CorruptionKind::ChecksumMismatch,
            };
            return finish_replay(projection, error, degrade_on_error);
        }

        let event: JournalEvent =
            serde_json::from_slice(payload).map_err(|error| ReplayError::CorruptRecord {
                offset: record_start,
                kind: CorruptionKind::InvalidJson(error.to_string()),
            })?;
        projection.apply(event);
        offset += payload_len;
    }

    Ok(projection)
}

fn finish_replay(
    mut projection: RuntimeProjection,
    error: ReplayError,
    degrade_on_error: bool,
) -> Result<RuntimeProjection, ReplayError> {
    if degrade_on_error {
        projection.degraded = Some(error);
        Ok(projection)
    } else {
        Err(error)
    }
}

impl RuntimeProjection {
    fn apply(&mut self, event: JournalEvent) {
        match event {
            JournalEvent::LaunchRequested {
                exec_id,
                original_command,
                mode,
            } => {
                let execution = self.ensure_execution(exec_id, original_command.clone());
                execution.command = original_command.clone();
                execution.original_command = original_command;
                execution.mode = Some(mode);
                execution.state = ProjectionState::Unknown;
                execution.observed_state = ProjectionState::Unknown;
                push_lifecycle(&mut execution.lifecycle, LifecycleStage::Requested);
            }
            JournalEvent::LaunchPolicyEvaluated {
                exec_id,
                rewritten_command,
                outcome,
            } => {
                let execution = self.ensure_execution(exec_id, String::new());
                execution.rewritten_command = rewritten_command;
                execution.policy_outcome = Some(outcome);
            }
            JournalEvent::LaunchAdmitted { exec_id, mode } => {
                let execution = self.ensure_execution(exec_id, String::new());
                execution.mode = Some(mode);
                push_lifecycle(&mut execution.lifecycle, LifecycleStage::Admitted);
            }
            JournalEvent::ProcessSpawned {
                exec_id,
                state,
                mode,
                ownership,
                stdout,
                stderr,
            } => {
                let execution = self.ensure_execution(exec_id, String::new());
                execution.state = state;
                execution.observed_state = execution.state.clone();
                execution.mode = Some(mode);
                execution.ownership = Some(ownership);
                execution.stdout = stdout;
                execution.stderr = stderr;
                push_lifecycle(&mut execution.lifecycle, LifecycleStage::Spawned);
            }
            JournalEvent::ExecutionRegistered {
                exec_id,
                state,
                command,
                stdout,
                stderr,
            } => {
                let execution = self.ensure_execution(exec_id, command);
                execution.state = state;
                execution.observed_state = execution.state.clone();
                execution.stdout = stdout;
                execution.stderr = stderr;
            }
            JournalEvent::ExecutionStateUpdated { exec_id, state } => {
                if let Some(execution) = self.executions.get_mut(exec_id.as_str()) {
                    execution.state = state.clone();
                    execution.observed_state = state.clone();
                    if state == ProjectionState::Exited {
                        push_lifecycle(&mut execution.lifecycle, LifecycleStage::Exited);
                    }
                }
            }
            JournalEvent::ServiceObserved { exec_id, service } => {
                let execution = self.ensure_execution(exec_id.clone(), String::new());
                execution.observed_state = ProjectionState::Service;
                execution.state = execution
                    .service_override
                    .clone()
                    .unwrap_or(ProjectionState::Service);
                self.services
                    .entry(exec_id.as_str().to_string())
                    .or_default()
                    .insert(service.name.clone(), service);
            }
            JournalEvent::ServiceOverrideApplied { exec_id, state } => {
                let execution = self.ensure_execution(exec_id, String::new());
                execution.service_override = Some(state.clone());
                execution.state = state;
            }
            JournalEvent::PortObserved { exec_id, port } => {
                self.ports
                    .entry(exec_id.as_str().to_string())
                    .or_default()
                    .insert(port.port_id.clone(), port);
            }
            JournalEvent::GhostStateRecorded { exec_id, ghost } => {
                self.ghosts.insert(exec_id.as_str().to_string(), ghost);
            }
            JournalEvent::ResourceGovernanceRecorded { exec_id, snapshot } => {
                let execution = self.ensure_execution(exec_id, String::new());
                execution.resource_governance = Some(snapshot);
            }
            JournalEvent::HistorySnapshotRecorded {
                exec_id,
                env_snapshot,
                manifest,
            } => {
                let key = exec_id.as_str().to_string();
                let stdout = manifest
                    .artifacts
                    .stdout
                    .as_ref()
                    .map(|artifact| artifact.blob.clone());
                let stderr = manifest
                    .artifacts
                    .stderr
                    .as_ref()
                    .map(|artifact| artifact.blob.clone());
                {
                    let execution = self.ensure_execution(exec_id, String::new());
                    execution.stdout = stdout;
                    execution.stderr = stderr;
                }
                self.env_snapshots.insert(key.clone(), env_snapshot);
                self.history_manifests.insert(key, *manifest);
            }
        }
    }

    fn ensure_execution(&mut self, exec_id: ExecutionId, command: String) -> &mut ExecutionView {
        let key = exec_id.as_str().to_string();
        self.executions.entry(key).or_insert_with(|| ExecutionView {
            exec_id,
            state: ProjectionState::Unknown,
            observed_state: ProjectionState::Unknown,
            command: command.clone(),
            original_command: command,
            rewritten_command: None,
            policy_outcome: None,
            stdout: None,
            stderr: None,
            mode: None,
            lifecycle: Vec::new(),
            ownership: None,
            resource_governance: None,
            service_override: None,
        })
    }

    fn mark_execution_live(&mut self, exec_id: &str) {
        let Some(execution) = self.executions.get_mut(exec_id) else {
            return;
        };

        let observed_state = if self
            .services
            .get(exec_id)
            .map(|services| !services.is_empty())
            .unwrap_or(false)
        {
            ProjectionState::Service
        } else {
            ProjectionState::Managed
        };

        execution.observed_state = observed_state.clone();
        execution.state = if observed_state == ProjectionState::Service {
            execution
                .service_override
                .clone()
                .unwrap_or(ProjectionState::Service)
        } else if execution.state == ProjectionState::ShortTask {
            ProjectionState::ShortTask
        } else {
            ProjectionState::Managed
        };
        self.ghosts.remove(exec_id);
    }

    fn set_uncertain_state(&mut self, exec_id: &str, state: ProjectionState, detail: &str) {
        let Some(execution) = self.executions.get_mut(exec_id) else {
            return;
        };

        execution.state = state.clone();
        execution.observed_state = state.clone();
        self.ghosts.insert(
            exec_id.to_string(),
            GhostView {
                state,
                detail: detail.to_string(),
            },
        );
    }

    fn clear_ghost(&mut self, exec_id: &str) {
        self.ghosts.remove(exec_id);
    }
}

fn push_lifecycle(stages: &mut Vec<LifecycleStage>, stage: LifecycleStage) {
    if !stages.contains(&stage) {
        stages.push(stage);
    }
}

fn build_environment_snapshot(
    exec_id: &ExecutionId,
    environment: &[(String, String)],
) -> EnvironmentSnapshotRecord {
    let mut redaction = RedactionMarker::Plaintext;
    let entries = environment
        .iter()
        .enumerate()
        .map(|(index, (name, value))| {
            if is_secret_name(name) {
                redaction = RedactionMarker::Redacted;
                EnvironmentEntryRecord {
                    name: format!("redacted-env-{index}"),
                    value: None,
                    name_redaction: RedactionMarker::Redacted,
                    value_redaction: RedactionMarker::Omitted,
                }
            } else {
                EnvironmentEntryRecord {
                    name: name.clone(),
                    value: Some(value.clone()),
                    name_redaction: RedactionMarker::Plaintext,
                    value_redaction: RedactionMarker::Plaintext,
                }
            }
        })
        .collect();

    EnvironmentSnapshotRecord {
        record_id: format!("env-{}", exec_id.as_str()),
        redaction,
        retention: RetentionClass::MetadataLongLived,
        entries,
    }
}

fn build_history_snapshot_manifest(
    exec_id: &ExecutionId,
    env_snapshot: &EnvironmentSnapshotRecord,
    input: HistorySnapshotManifestInput<'_>,
) -> HistorySnapshotManifest {
    HistorySnapshotManifest {
        snapshot_id: format!("snapshot-{}", exec_id.as_str()),
        schema_version: HISTORY_SNAPSHOT_SCHEMA_VERSION,
        captured_at_epoch_ms: epoch_millis(),
        execution: SnapshotExecutionIdentity {
            exec_id: exec_id.clone(),
            lineage_root_exec_id: exec_id.clone(),
        },
        intent: SnapshotIntentRecord {
            original_command: input.persisted_original_command.to_string(),
        },
        launch: SnapshotLaunchRecord {
            rewritten_command: input.persisted_rewritten_command,
        },
        observed: input.observed,
        environment: SnapshotEnvironmentReference {
            record_id: env_snapshot.record_id.clone(),
            redaction: env_snapshot.redaction.clone(),
        },
        artifacts: input.artifacts,
        host: SnapshotHostMetadata {
            platform: std::env::consts::OS.to_string(),
            arch: std::env::consts::ARCH.to_string(),
        },
        retention: RetentionClass::MetadataLongLived,
    }
}

fn redact_command_string_for_persistence(command: &str) -> String {
    let mut previous_secret_flag = false;
    command
        .split_whitespace()
        .map(|part| {
            if previous_secret_flag {
                previous_secret_flag = false;
                return "<redacted>".to_string();
            }

            if let Some((name, _)) = part.split_once('=') {
                if is_secret_name(name) {
                    return format!("{name}=<redacted>");
                }
            }

            if is_secret_flag(part) {
                previous_secret_flag = true;
            }

            part.to_string()
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn is_secret_flag(part: &str) -> bool {
    matches!(
        part,
        "--token" | "--password" | "--secret" | "--api-key" | "--auth-token" | "-p"
    )
}

fn is_secret_name(name: &str) -> bool {
    let upper = name.to_ascii_uppercase();
    [
        "TOKEN",
        "SECRET",
        "PASSWORD",
        "PASS",
        "API_KEY",
        "ACCESS_KEY",
        "SECRET_KEY",
        "AUTH",
        "COOKIE",
        "SESSION",
    ]
    .iter()
    .any(|needle| upper.contains(needle))
}

fn epoch_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

fn sha256_hex(bytes: &[u8]) -> String {
    let mut digest = Sha256::new();
    digest.update(bytes);
    hex_digest(digest.finalize())
}

fn hex_digest(bytes: impl AsRef<[u8]>) -> String {
    bytes
        .as_ref()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn ownership_matches(ownership: &RuntimeOwnership, observed: &ObservedProcess) -> bool {
    if ownership.root_pid != observed.root_pid {
        return false;
    }

    if ownership.process_group_id != 0 && ownership.process_group_id != observed.process_group_id {
        return false;
    }

    if let Some(start_time_ticks) = ownership.start_time_ticks {
        if observed.start_time_ticks != Some(start_time_ticks) {
            return false;
        }
    }

    if let (Some(expected_session), Some(observed_session)) =
        (ownership.session_id, observed.session_id)
    {
        if expected_session != observed_session {
            return false;
        }
    }

    true
}

fn projection_state_name(state: &ProjectionState) -> &'static str {
    match state {
        ProjectionState::Managed => "managed",
        ProjectionState::Service => "service",
        ProjectionState::ShortTask => "short_task",
        ProjectionState::Missing => "missing",
        ProjectionState::Orphaned => "orphaned",
        ProjectionState::Escaped => "escaped",
        ProjectionState::Detached => "detached",
        ProjectionState::Exited => "exited",
        ProjectionState::Unknown => "unknown",
    }
}

#[derive(Debug)]
struct PolicyEvaluation {
    effective_spec: ManagedLaunchSpec,
    rewritten_command: Option<String>,
    outcome: LaunchPolicyOutcome,
}

fn evaluate_launch_policy(spec: ManagedLaunchSpec) -> Result<PolicyEvaluation, ManagedExecError> {
    if spec.program.file_name().and_then(|name| name.to_str()) != Some("rm") {
        return Ok(PolicyEvaluation {
            effective_spec: spec,
            rewritten_command: None,
            outcome: LaunchPolicyOutcome::AllowedAsRequested {
                policy: RM_POLICY_NAME.to_string(),
                reason: "command is not a direct rm invocation".to_string(),
            },
        });
    }

    rewrite_rm_launch(spec)
}

fn rewrite_rm_launch(spec: ManagedLaunchSpec) -> Result<PolicyEvaluation, ManagedExecError> {
    let operands = parse_rm_operands(&spec.args)?;
    if operands.len() != 1 {
        return deny_rm(
            spec,
            "rm safe delete backend only supports exactly one deterministic operand".to_string(),
        );
    }

    let operand = &operands[0];
    if contains_ambiguous_shell_syntax(operand) {
        return deny_rm(
            spec,
            format!("operand {operand} contains ambiguous shell metacharacters"),
        );
    }

    let resolved = resolve_operand(operand).map_err(|reason| ManagedExecError::PolicyDenied {
        policy: RM_POLICY_NAME.to_string(),
        reason,
    })?;

    if is_protected_path(&resolved).map_err(|error| ManagedExecError::Io(error.to_string()))? {
        return deny_rm(
            spec,
            format!(
                "resolved operand {} targets a protected path",
                resolved.display()
            ),
        );
    }

    let rewritten_spec = safe_delete_rewrite(spec, &resolved)?;
    let rewritten_command = rewritten_spec.rendered_command();
    Ok(PolicyEvaluation {
        effective_spec: rewritten_spec,
        rewritten_command: Some(rewritten_command),
        outcome: LaunchPolicyOutcome::Rewritten {
            policy: RM_POLICY_NAME.to_string(),
            reason: "direct rm operand was deterministically rewritten to safe delete".to_string(),
        },
    })
}

fn deny_rm(spec: ManagedLaunchSpec, reason: String) -> Result<PolicyEvaluation, ManagedExecError> {
    Ok(PolicyEvaluation {
        effective_spec: spec,
        rewritten_command: None,
        outcome: LaunchPolicyOutcome::Denied {
            policy: RM_POLICY_NAME.to_string(),
            reason,
        },
    })
}

fn parse_rm_operands(args: &[String]) -> Result<Vec<String>, ManagedExecError> {
    let mut operands = Vec::new();
    let mut parsing_flags = true;

    for arg in args {
        if parsing_flags {
            if arg == "--" {
                parsing_flags = false;
                continue;
            }

            if arg.starts_with('-') && arg != "-" {
                validate_rm_flag(arg)?;
                continue;
            }

            parsing_flags = false;
        }

        operands.push(arg.clone());
    }

    if operands.is_empty() {
        return Err(ManagedExecError::PolicyDenied {
            policy: RM_POLICY_NAME.to_string(),
            reason: "rm command did not include any operands".to_string(),
        });
    }

    Ok(operands)
}

fn validate_rm_flag(flag: &str) -> Result<(), ManagedExecError> {
    let allowed = match flag {
        "--force" | "--recursive" => true,
        _ if flag.starts_with('-') && !flag.starts_with("--") => flag
            .trim_start_matches('-')
            .chars()
            .all(|ch| matches!(ch, 'r' | 'R' | 'f')),
        _ => false,
    };

    if allowed {
        Ok(())
    } else {
        Err(ManagedExecError::PolicyDenied {
            policy: RM_POLICY_NAME.to_string(),
            reason: format!("rm flag {flag} is not supported by the safe delete adapter"),
        })
    }
}

fn contains_ambiguous_shell_syntax(operand: &str) -> bool {
    operand.chars().any(|ch| {
        matches!(
            ch,
            '*' | '?'
                | '['
                | ']'
                | '{'
                | '}'
                | '~'
                | '$'
                | '('
                | ')'
                | '|'
                | '&'
                | ';'
                | '<'
                | '>'
                | '`'
                | '\n'
                | '\r'
        )
    })
}

fn resolve_operand(operand: &str) -> Result<std::path::PathBuf, String> {
    let candidate = if Path::new(operand).is_absolute() {
        std::path::PathBuf::from(operand)
    } else {
        std::env::current_dir()
            .map_err(|error| format!("failed to read current working directory: {error}"))?
            .join(operand)
    };

    fs::canonicalize(&candidate).map_err(|error| {
        format!("operand {operand} could not be resolved deterministically: {error}")
    })
}

fn is_protected_path(path: &Path) -> io::Result<bool> {
    if path == Path::new("/") || is_mount_point(path)? {
        return Ok(true);
    }

    for protected in protected_paths() {
        if path == protected || protected.starts_with(path) {
            return Ok(true);
        }
    }

    Ok(false)
}

fn protected_paths() -> Vec<std::path::PathBuf> {
    let mut paths = Vec::new();
    paths.push(std::path::PathBuf::from("/"));

    if let Some(home) = std::env::var_os("HOME") {
        paths.push(std::path::PathBuf::from(home));
    }

    if let Some(repo_root) = repo_root() {
        paths.push(repo_root);
    }

    paths
}

fn repo_root() -> Option<std::path::PathBuf> {
    let manifest_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest_dir.parent()?.parent().map(Path::to_path_buf)
}

fn is_mount_point(path: &Path) -> io::Result<bool> {
    #[cfg(unix)]
    {
        if path == Path::new("/") {
            return Ok(true);
        }

        let Some(parent) = path.parent() else {
            return Ok(true);
        };
        let metadata = fs::metadata(path)?;
        let parent_metadata = fs::metadata(parent)?;
        Ok(metadata.dev() != parent_metadata.dev())
    }

    #[cfg(not(unix))]
    {
        let _ = path;
        Ok(false)
    }
}

fn safe_delete_rewrite(
    spec: ManagedLaunchSpec,
    resolved: &Path,
) -> Result<ManagedLaunchSpec, ManagedExecError> {
    let parent = resolved
        .parent()
        .ok_or_else(|| ManagedExecError::PolicyDenied {
            policy: RM_POLICY_NAME.to_string(),
            reason: format!(
                "resolved operand {} has no parent directory",
                resolved.display()
            ),
        })?;
    let file_name = resolved
        .file_name()
        .ok_or_else(|| ManagedExecError::PolicyDenied {
            policy: RM_POLICY_NAME.to_string(),
            reason: format!(
                "resolved operand {} has no terminal path segment",
                resolved.display()
            ),
        })?;

    let trash_dir = parent.join(".execmanager-trash");
    fs::create_dir_all(&trash_dir)?;

    let mut destination = trash_dir.join(format!(
        "{}-{}",
        spec.exec_id.as_str(),
        file_name.to_string_lossy()
    ));
    let mut suffix = 1u32;
    while destination.exists() {
        destination = trash_dir.join(format!(
            "{}-{}-{suffix}",
            spec.exec_id.as_str(),
            file_name.to_string_lossy()
        ));
        suffix += 1;
    }

    Ok(ManagedLaunchSpec::new(
        spec.exec_id,
        "/bin/mv",
        vec![
            "--".to_string(),
            resolved.display().to_string(),
            destination.display().to_string(),
        ],
        spec.mode,
    )
    .with_original_command(spec.original_command))
}

fn observed_listeners_for_pid(pid: u32) -> io::Result<Vec<ObservedListener>> {
    let socket_inodes = socket_inodes_for_pid(pid)?;
    if socket_inodes.is_empty() {
        return Ok(Vec::new());
    }

    let mut listeners = listener_rows_from_proc("/proc/net/tcp", &socket_inodes, "tcp")?;
    listeners.extend(listener_rows_from_proc(
        "/proc/net/tcp6",
        &socket_inodes,
        "tcp",
    )?);
    listeners.sort_by_key(|listener| listener.port);
    listeners.dedup_by(|left, right| left.port_id == right.port_id);
    Ok(listeners)
}

fn socket_inodes_for_pid(pid: u32) -> io::Result<HashSet<u64>> {
    let mut inodes = HashSet::new();
    let fd_dir = PathBuf::from(format!("/proc/{pid}/fd"));
    for entry in fs::read_dir(fd_dir)? {
        let entry = entry?;
        let target = fs::read_link(entry.path())?;
        let target = target.to_string_lossy();
        if let Some(raw_inode) = target
            .strip_prefix("socket:[")
            .and_then(|value| value.strip_suffix(']'))
        {
            if let Ok(inode) = raw_inode.parse::<u64>() {
                inodes.insert(inode);
            }
        }
    }
    Ok(inodes)
}

fn listener_rows_from_proc(
    table_path: &str,
    socket_inodes: &HashSet<u64>,
    protocol: &str,
) -> io::Result<Vec<ObservedListener>> {
    let contents = match fs::read_to_string(table_path) {
        Ok(contents) => contents,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error),
    };

    let mut listeners = Vec::new();
    for line in contents.lines().skip(1) {
        let columns: Vec<_> = line.split_whitespace().collect();
        if columns.len() <= 9 || columns[3] != "0A" {
            continue;
        }

        let Some(inode) = columns[9].parse::<u64>().ok() else {
            continue;
        };
        if !socket_inodes.contains(&inode) {
            continue;
        }

        let Some(local_address) = columns[1].split(':').nth(1) else {
            continue;
        };
        let Ok(port) = u16::from_str_radix(local_address, 16) else {
            continue;
        };

        listeners.push(ObservedListener {
            port_id: format!("{protocol}:{port}"),
            port,
            protocol: protocol.to_string(),
        });
    }

    Ok(listeners)
}

fn process_group_id_for(pid: u32) -> Result<u32, ManagedExecError> {
    let pgid = unsafe { libc::getpgid(pid as libc::pid_t) };
    if pgid < 0 {
        Err(io::Error::last_os_error().into())
    } else {
        Ok(pgid as u32)
    }
}

fn session_id_for(pid: u32) -> Option<u32> {
    let sid = unsafe { libc::getsid(pid as libc::pid_t) };
    if sid < 0 { None } else { Some(sid as u32) }
}

fn process_start_time_ticks(pid: u32) -> Result<Option<u64>, ManagedExecError> {
    #[cfg(target_os = "linux")]
    {
        let path = PathBuf::from(format!("/proc/{pid}/stat"));
        let mut stat = String::new();
        File::open(path)?.read_to_string(&mut stat)?;
        let end = stat.rfind(')').ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "failed to parse /proc stat command boundary",
            )
        })?;
        let remainder = stat[end + 2..].trim();
        let fields: Vec<&str> = remainder.split_whitespace().collect();
        let start_time_index = 19usize;
        let start_time = fields
            .get(start_time_index)
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    "missing /proc stat start time field",
                )
            })?
            .parse::<u64>()
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error.to_string()))?;
        Ok(Some(start_time))
    }

    #[cfg(not(target_os = "linux"))]
    {
        let _ = pid;
        Ok(None)
    }
}

fn signal_process_group(
    process_group_id: u32,
    signal: libc::c_int,
) -> Result<(), ManagedCleanupError> {
    if unsafe { libc::kill(-(process_group_id as libc::pid_t), signal) } == -1 {
        let error = io::Error::last_os_error();
        if error.kind() == io::ErrorKind::NotFound {
            return Ok(());
        }
        return Err(error.into());
    }
    Ok(())
}

fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = !0u32;
    for byte in bytes {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg() & 0xEDB8_8320;
            crc = (crc >> 1) ^ mask;
        }
    }
    !crc
}

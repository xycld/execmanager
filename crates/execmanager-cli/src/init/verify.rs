use std::{
    io, thread,
    time::{Duration, Instant},
};

use crate::{app_dirs::AppDirs, commands::ServiceCommand, service::run_service_command, CliError};

const DAEMON_READINESS_TIMEOUT: Duration = Duration::from_secs(2);
const DAEMON_READINESS_POLL_INTERVAL: Duration = Duration::from_millis(100);

pub fn start_service_and_verify_daemon_readiness(dirs: &AppDirs) -> Result<(), CliError> {
    start_service_and_verify_daemon_readiness_with(
        dirs,
        start_execmanager_service,
        verify_daemon_readiness,
    )
}

pub fn start_service_and_verify_daemon_readiness_with<Start, Verify>(
    dirs: &AppDirs,
    mut start_service: Start,
    mut verify_readiness: Verify,
) -> Result<(), CliError>
where
    Start: FnMut(&AppDirs) -> Result<(), CliError>,
    Verify: FnMut(&AppDirs) -> Result<(), CliError>,
{
    start_service(dirs)?;
    wait_for_daemon_readiness(dirs, &mut verify_readiness)
}

pub fn verify_daemon_readiness(dirs: &AppDirs) -> Result<(), CliError> {
    let socket_path = daemon_socket_path(dirs);

    if !socket_path.exists() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("daemon socket is missing at {}", socket_path.display()),
        )
        .into());
    }

    run_probe(execmanager_daemon::probe_rpc_readiness(&socket_path)).map_err(Into::into)
}

fn daemon_socket_path(dirs: &AppDirs) -> std::path::PathBuf {
    dirs.runtime_dir.join("execmanager.sock")
}

fn start_execmanager_service(dirs: &AppDirs) -> Result<(), CliError> {
    run_service_command(dirs, ServiceCommand::Start).map(|_| ())
}

fn wait_for_daemon_readiness<Verify>(
    dirs: &AppDirs,
    verify_readiness: &mut Verify,
) -> Result<(), CliError>
where
    Verify: FnMut(&AppDirs) -> Result<(), CliError>,
{
    let deadline = Instant::now() + DAEMON_READINESS_TIMEOUT;

    loop {
        match verify_readiness(dirs) {
            Ok(()) => return Ok(()),
            Err(error) => {
                if Instant::now() >= deadline {
                    return Err(format!(
                        "timed out waiting for daemon readiness after {:?}: {}",
                        DAEMON_READINESS_TIMEOUT, error
                    )
                    .into());
                }
            }
        }

        thread::sleep(DAEMON_READINESS_POLL_INTERVAL);
    }
}

fn run_probe<F>(future: F) -> io::Result<()>
where
    F: std::future::Future<Output = io::Result<()>>,
{
    match tokio::runtime::Handle::try_current() {
        Ok(handle) => tokio::task::block_in_place(|| handle.block_on(future)),
        Err(_) => tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?
            .block_on(future),
    }
}

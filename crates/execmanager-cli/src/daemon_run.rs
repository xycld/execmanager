use std::future;

use tokio::fs;

use crate::{app_dirs::AppDirs, CliError};

pub async fn run_daemon(dirs: &AppDirs) -> Result<(), CliError> {
    fs::create_dir_all(&dirs.config_dir).await?;
    fs::create_dir_all(&dirs.runtime_dir).await?;
    fs::create_dir_all(&dirs.state_dir).await?;

    let config = execmanager_daemon::DaemonRpcConfig::new(
        dirs.runtime_dir.join("execmanager.sock"),
        dirs.state_dir.join("events.journal"),
    );
    let server = execmanager_daemon::spawn_rpc_server(config)?;

    future::pending::<()>().await;
    drop(server);
    Ok(())
}

use std::io;

use crate::{
    app_dirs::AppDirs,
    init::verify::verify_daemon_readiness,
    metadata::{InitMetadata, InstallState},
    recovery::RecoveryMetadata,
    CliError,
};

pub fn run_doctor(dirs: &AppDirs) -> Result<String, CliError> {
    let metadata = InitMetadata::load(dirs)?;
    let mut lines = Vec::new();
    let failed_partial = matches!(metadata.install_state, InstallState::FailedPartial);

    lines.push(format!(
        "install state: {}",
        install_state_label(&metadata.install_state)
    ));

    if !metadata.initialized && !failed_partial {
        lines.push("init metadata missing -> run `execmanager init`".to_string());
    }

    if failed_partial {
        let recovery = load_recovery_metadata(dirs)?;
        let restore_available = matches!(recovery.as_ref(), Some(data) if data.fully_restorable);
        let restore_available_label = if restore_available { "yes" } else { "no" };

        lines.push(format!("restore available: {restore_available_label}"));

        if restore_available {
            lines.push(
                "partial install detected -> run `execmanager uninstall --restore` to roll back or `execmanager init` to retry installation"
                    .to_string(),
            );
        } else {
            lines.push(
                "partial install detected -> run `execmanager init` to repair the installation state"
                    .to_string(),
            );
        }
    }

    if !failed_partial {
        if let Err(error) = verify_daemon_readiness(dirs) {
            lines.push(format!(
                "daemon not reachable ({error}) -> re-run `execmanager init` to repair setup or launch `execmanager daemon run` directly"
            ));
        }
    }

    if lines.is_empty() {
        lines.push("doctor: ok".to_string());
    }

    Ok(lines.join("\n"))
}

fn load_recovery_metadata(dirs: &AppDirs) -> Result<Option<RecoveryMetadata>, CliError> {
    match RecoveryMetadata::load(dirs) {
        Ok(metadata) => Ok(Some(metadata)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
}

fn install_state_label(state: &InstallState) -> &'static str {
    match state {
        InstallState::NotInstalled => "not-installed",
        InstallState::Installing => "installing",
        InstallState::Installed => "installed",
        InstallState::RepairNeeded => "repair-needed",
        InstallState::FailedPartial => "failed-partial",
    }
}

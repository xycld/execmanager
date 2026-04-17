use std::fs;

use crate::{
    adapters::{Adapter, KimiAdapter},
    app_dirs::AppDirs,
    recovery::{HookInstallMode, RecoveryMetadata},
    service::{current_user_service_definition_path, service_definition_path_for_root},
    CliError,
};

const RECOVERY_METADATA_FILE_NAME: &str = "recovery.json";

pub fn run_uninstall(dirs: &AppDirs) -> Result<(), CliError> {
    safe_uninstall_hook(dirs)?;
    remove_managed_file(managed_service_definition_path(dirs)?)?;
    remove_common_uninstall_artifacts(dirs)?;
    Ok(())
}

pub fn run_restore_uninstall(dirs: &AppDirs) -> Result<String, CliError> {
    let recovery = match RecoveryMetadata::load(dirs) {
        Ok(recovery) => recovery,
        Err(_) => {
            run_uninstall(dirs)?;
            return Ok(format!(
                "recovery metadata was unavailable, so execmanager performed safe removal in {}",
                dirs.config_dir.display()
            ));
        }
    };

    let mut limitations = Vec::new();

    restore_or_remove_hook(dirs, &recovery, &mut limitations)?;
    restore_or_remove_service(dirs, &recovery, &mut limitations)?;
    remove_common_uninstall_artifacts(dirs)?;

    let mut message = format!(
        "restored original artifacts where possible and removed execmanager-managed state from {}",
        dirs.config_dir.display()
    );

    if !limitations.is_empty() {
        message.push_str("; ");
        message.push_str(&limitations.join("; "));
    }

    Ok(message)
}

fn restore_or_remove_hook(
    dirs: &AppDirs,
    recovery: &RecoveryMetadata,
    limitations: &mut Vec<String>,
) -> Result<(), CliError> {
    match recovery.selected_adapter.as_str() {
        "kimi" => match recovery.hook_backup_contents.as_deref() {
            Some(contents) => {
                let hook_path = kimi_adapter_for(dirs)?.plan_hook_install().hook_path;
                write_restored_file(&hook_path, contents)
            }
            None if recovery.hook_install_mode == HookInstallMode::NewFile => {
                safe_uninstall_hook(dirs)
            }
            None => {
                safe_uninstall_hook(dirs)?;
                limitations.push(
                    "hook backup metadata was unavailable, so the hook fell back to safe removal"
                        .to_string(),
                );
                Ok(())
            }
        },
        other => {
            safe_uninstall_hook(dirs)?;
            limitations.push(format!(
                "adapter `{other}` is not restorable here, so the hook fell back to safe removal"
            ));
            Ok(())
        }
    }
}

fn restore_or_remove_service(
    dirs: &AppDirs,
    recovery: &RecoveryMetadata,
    limitations: &mut Vec<String>,
) -> Result<(), CliError> {
    let expected_service_path = managed_service_definition_path(dirs)?;

    if recovery.service_definition_path != expected_service_path {
        remove_managed_file(expected_service_path.clone())?;
        limitations.push(format!(
            "recovery metadata service path {} did not match the expected managed service path {}, so the service fell back to safe removal at the validated managed path only",
            recovery.service_definition_path.display(),
            expected_service_path.display()
        ));
        return Ok(());
    }

    if let Some(contents) = recovery.service_definition_backup_contents.as_deref() {
        return write_restored_file(&expected_service_path, contents);
    }

    if recovery.service_previously_present {
        remove_managed_file(expected_service_path)?;
        limitations.push(
            "service definition backup metadata was unavailable, so the service file fell back to safe removal"
                .to_string(),
        );
        return Ok(());
    }

    remove_managed_file(expected_service_path)
}

fn safe_uninstall_hook(dirs: &AppDirs) -> Result<(), CliError> {
    kimi_adapter_for(dirs)?.uninstall_managed_hook()?;
    Ok(())
}

fn kimi_adapter_for(dirs: &AppDirs) -> Result<KimiAdapter, CliError> {
    if let Some(root) = test_root(dirs) {
        return Ok(KimiAdapter::for_test(root.join("kimi-hook.sh")));
    }

    KimiAdapter::new().map_err(Into::into)
}

fn remove_common_uninstall_artifacts(dirs: &AppDirs) -> Result<(), CliError> {
    remove_managed_file(dirs.metadata_file())?;
    remove_managed_file(recovery_metadata_path(dirs))?;
    remove_managed_file(dirs.runtime_dir.join("execmanager.sock"))?;
    remove_managed_file(dirs.state_dir.join("events.journal"))?;
    Ok(())
}

fn recovery_metadata_path(dirs: &AppDirs) -> std::path::PathBuf {
    dirs.config_dir.join(RECOVERY_METADATA_FILE_NAME)
}

fn managed_service_definition_path(dirs: &AppDirs) -> Result<std::path::PathBuf, CliError> {
    if let Some(root) = test_root(dirs) {
        return Ok(service_definition_path_for_root(root));
    }

    current_user_service_definition_path()
}

fn test_root(dirs: &AppDirs) -> Option<&std::path::Path> {
    let config_parent = dirs.config_dir.parent()?;
    let runtime_parent = dirs.runtime_dir.parent()?;
    let state_parent = dirs.state_dir.parent()?;

    (dirs.config_dir.file_name()? == "config"
        && dirs.runtime_dir.file_name()? == "runtime"
        && dirs.state_dir.file_name()? == "state"
        && config_parent == runtime_parent
        && config_parent == state_parent)
        .then_some(config_parent)
}

fn remove_managed_file(path: std::path::PathBuf) -> Result<(), CliError> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

fn write_restored_file(path: &std::path::Path, contents: &str) -> Result<(), CliError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    fs::write(path, contents)?;
    Ok(())
}

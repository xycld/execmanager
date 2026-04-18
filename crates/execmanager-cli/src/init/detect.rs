use std::{
    fs,
    path::{Path, PathBuf},
};

use crate::{
    adapters::{Adapter, KimiAdapter},
    app_dirs::AppDirs,
    service::{
        current_user_service_definition_path, service_definition_path_for_root, ServiceKind,
    },
    CliError,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct InitContext {
    pub(crate) dirs: AppDirs,
    pub(crate) adapter: KimiAdapter,
    pub(crate) adapter_key: String,
    pub(crate) service_kind: ServiceKind,
    pub(crate) service_label: String,
    pub(crate) service_definition_path: PathBuf,
    pub(crate) service_previously_present: bool,
    pub(crate) service_definition_backup_contents: Option<String>,
    pub(crate) execmanager_path: PathBuf,
}

pub(crate) fn detect_for_test_root(root: &Path) -> Result<InitContext, CliError> {
    let dirs = AppDirs::for_test(root);
    let adapter = KimiAdapter::for_test(root.join("kimi-hook.sh"));

    detect_context(
        dirs,
        adapter,
        service_definition_path_for_root(root),
        root.join("execmanager"),
    )
}

pub(crate) fn detect_for_current_user(
    dirs: AppDirs,
    execmanager_path: PathBuf,
) -> Result<InitContext, CliError> {
    let adapter = KimiAdapter::new()?;

    detect_context(
        dirs,
        adapter,
        current_user_service_definition_path()?,
        execmanager_path,
    )
}

fn detect_context(
    dirs: AppDirs,
    adapter: KimiAdapter,
    service_definition_path: PathBuf,
    execmanager_path: PathBuf,
) -> Result<InitContext, CliError> {
    let (service_kind, service_label) = detect_service_kind();
    let (service_previously_present, service_definition_backup_contents) =
        read_service_definition_snapshot(&service_definition_path);

    Ok(InitContext {
        dirs,
        adapter_key: adapter.key().to_string(),
        adapter,
        service_kind,
        service_label: service_label.to_string(),
        service_previously_present,
        service_definition_backup_contents,
        service_definition_path,
        execmanager_path,
    })
}

fn read_service_definition_snapshot(service_definition_path: &Path) -> (bool, Option<String>) {
    match fs::read_to_string(service_definition_path) {
        Ok(contents) => (true, Some(contents)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => (false, None),
        Err(_) => (fs::metadata(service_definition_path).is_ok(), None),
    }
}

#[cfg(target_os = "linux")]
fn detect_service_kind() -> (ServiceKind, &'static str) {
    (ServiceKind::SystemdUser, "systemd --user")
}

#[cfg(target_os = "macos")]
fn detect_service_kind() -> (ServiceKind, &'static str) {
    (ServiceKind::LaunchAgent, "LaunchAgent")
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn detect_service_kind() -> (ServiceKind, &'static str) {
    (ServiceKind::SystemdUser, "systemd --user")
}

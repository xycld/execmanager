use crate::{
    app_dirs::AppDirs,
    metadata::{InitMetadata, InstallState},
    CliError,
};

pub fn render_status(dirs: &AppDirs) -> Result<String, CliError> {
    let metadata = InitMetadata::load(dirs)?;
    let initialized = if metadata.initialized { "yes" } else { "no" };
    let adapter = metadata.selected_adapter.as_deref().unwrap_or("unknown");
    let service = metadata.service_kind.as_deref().unwrap_or("unknown");
    let install_state = install_state_label(&metadata.install_state);
    let install_version = metadata.install_version.as_deref().unwrap_or("unknown");

    Ok(format!(
        concat!(
            "initialized: {}\n",
            "adapter: {}\n",
            "service: {}\n",
            "install state: {}\n",
            "install version: {}\n",
            "config dir: {}\n",
            "runtime dir: {}\n",
            "state dir: {}"
        ),
        initialized,
        adapter,
        service,
        install_state,
        install_version,
        dirs.config_dir.display(),
        dirs.runtime_dir.display(),
        dirs.state_dir.display(),
    ))
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

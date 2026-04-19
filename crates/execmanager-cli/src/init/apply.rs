use std::fs;

use crate::{
    app_dirs::AppDirs,
    init::{verify::start_service_and_verify_daemon_readiness, InitPlan},
    metadata::{InitMetadata, InstallState},
    recovery::HookInstallOutcome,
    service::LaunchSpec,
    CliError,
};

pub fn apply_init_plan(plan: &InitPlan) -> Result<(), CliError> {
    apply_init_plan_with_daemon_stage(plan, start_service_and_verify_daemon_readiness)
}

pub fn apply_init_plan_with_daemon_stage<F>(
    plan: &InitPlan,
    daemon_stage: F,
) -> Result<(), CliError>
where
    F: FnOnce(&AppDirs) -> Result<(), CliError>,
{
    let context = &plan.context;

    write_install_metadata(plan, InstallState::Installing, false)?;

    fs::create_dir_all(&context.dirs.config_dir)?;
    fs::create_dir_all(&context.dirs.runtime_dir)?;
    fs::create_dir_all(&context.dirs.state_dir)?;

    let result = apply_install_transaction(plan, daemon_stage);

    match result {
        Ok(()) => {
            write_install_metadata(plan, InstallState::Installed, true)?;
            Ok(())
        }
        Err(error) => {
            write_install_metadata(plan, InstallState::FailedPartial, false)?;
            Err(error)
        }
    }
}

fn apply_install_transaction<F>(plan: &InitPlan, daemon_stage: F) -> Result<(), CliError>
where
    F: FnOnce(&AppDirs) -> Result<(), CliError>,
{
    let context = &plan.context;

    let hook_outcome = context.adapter.install_managed_hook_with_outcome()?;
    persist_recovery_metadata(plan, hook_outcome)?;

    register_service_definition(context)?;
    daemon_stage(&context.dirs)?;

    Ok(())
}

fn persist_recovery_metadata(
    plan: &InitPlan,
    hook_outcome: HookInstallOutcome,
) -> Result<(), CliError> {
    plan.recovery_metadata_for(hook_outcome.install_mode, hook_outcome.previous_contents)
        .store(&plan.context.dirs)?;
    Ok(())
}

fn write_install_metadata(
    plan: &InitPlan,
    install_state: InstallState,
    initialized: bool,
) -> Result<(), CliError> {
    InitMetadata {
        initialized,
        selected_adapter: Some(plan.context.adapter_key.clone()),
        service_kind: Some(plan.context.service_label.clone()),
        install_state,
        install_version: Some(install_version_for(&plan.context.execmanager_path)),
    }
    .store(&plan.context.dirs)?;

    Ok(())
}

fn install_version_for(execmanager_path: &std::path::Path) -> String {
    let base_version = env!("CARGO_PKG_VERSION");
    let install_channel_marker = execmanager_path
        .parent()
        .map(|parent| parent.join(".execmanager-install-channel"));

    match install_channel_marker
        .as_ref()
        .and_then(|path| fs::read_to_string(path).ok())
        .map(|contents| contents.trim().to_string())
        .as_deref()
    {
        Some("snapshot") => format!("{base_version}-snapshot"),
        _ => base_version.to_string(),
    }
}

fn register_service_definition(context: &super::InitContext) -> Result<(), CliError> {
    let Some(parent) = context.service_definition_path.parent() else {
        return Err("service definition path has no parent directory".into());
    };

    fs::create_dir_all(parent)?;

    let launch_spec = LaunchSpec {
        execmanager_path: context.execmanager_path.clone(),
        config_dir: context.dirs.config_dir.clone(),
        runtime_dir: context.dirs.runtime_dir.clone(),
        state_dir: context.dirs.state_dir.clone(),
    };

    let rendered = context.service_kind.render(&launch_spec);
    fs::write(&context.service_definition_path, rendered)?;

    Ok(())
}

pub mod adapters;
pub mod app_dirs;
pub mod commands;
pub mod daemon_run;
pub mod doctor;
pub mod init;
pub mod metadata;
mod persist;
pub mod recovery;
pub mod service;
pub mod status;
#[cfg(test)]
pub mod test_support;
pub mod uninstall;

use std::{
    io::{self, Write},
    path::Path,
};

use adapters::{Adapter, KimiAdapter};
use app_dirs::AppDirs;
use commands::{Command, HooksCommand, UninstallMode};
use doctor::run_doctor;
use init::{apply_init_plan, apply_init_plan_with_daemon_stage, build_current_user_init_plan, InitMode, InitPlan};
use metadata::InitMetadata;
use service::run_service_command_with_runner;
use status::render_status;
use uninstall::{run_restore_uninstall, run_uninstall};

pub type CliError = Box<dyn std::error::Error + Send + Sync>;

const SMART_ENTRY_INIT_GUIDANCE: &str = "ExecManager is not initialized. Run `execmanager init`.";
const NONINTERACTIVE_INIT_GUIDANCE: &str = "ExecManager init requires an interactive terminal. Re-run `execmanager init` from an interactive terminal to review and apply installation changes.";

pub fn run_init(dirs: &AppDirs, interactive: bool) -> Result<String, CliError> {
    run_init_with(dirs, interactive, confirm_install_apply, apply_init_plan)
}

pub async fn run_smart_entry(dirs: &AppDirs, interactive: bool) -> Result<String, CliError> {
    let metadata = InitMetadata::load(dirs)?;
    if !metadata.initialized && interactive {
        return run_init(dirs, true);
    }
    if !metadata.initialized {
        return Ok(SMART_ENTRY_INIT_GUIDANCE.to_string());
    }

    render_status(dirs)
}

pub async fn run_current_user_command(
    command: Command,
    interactive: bool,
) -> Result<String, CliError> {
    match command {
        Command::SmartEntry => {
            let dirs = AppDirs::for_current_user()?;
            run_smart_entry(&dirs, interactive).await
        }
        Command::Init => {
            let dirs = AppDirs::for_current_user()?;
            run_init(&dirs, interactive)
        }
        Command::Status => {
            let dirs = AppDirs::for_current_user()?;
            render_status(&dirs)
        }
        Command::Doctor => {
            let dirs = AppDirs::for_current_user()?;
            run_doctor(&dirs)
        }
        Command::Service(service_command) => {
            let dirs = AppDirs::for_current_user()?;
            service::run_service_command(&dirs, service_command)
        }
        Command::Hooks(hooks_command) => {
            let dirs = AppDirs::for_current_user()?;
            let adapter = resolve_selected_adapter(&dirs, || KimiAdapter::new().map_err(Into::into))?;
            run_hooks_command(&adapter, hooks_command)
        }
        Command::Uninstall(mode) => {
            let dirs = AppDirs::for_current_user()?;
            run_uninstall_command(&dirs, mode)
        }
        Command::DaemonRun => {
            let dirs = AppDirs::for_current_user()?;
            daemon_run::run_daemon(&dirs).await?;
            Ok(String::new())
        }
    }
}

pub fn run_for_test<P, I, S>(root: P, interactive: bool, args: I) -> Result<String, CliError>
where
    P: AsRef<Path>,
    I: IntoIterator<Item = S>,
    S: Into<String>,
{
    run_for_test_with_service_runner(root, interactive, args, |_| Ok(()))
}

pub fn run_for_test_with_service_runner<P, I, S, F>(
    root: P,
    interactive: bool,
    args: I,
    runner: F,
) -> Result<String, CliError>
where
    P: AsRef<Path>,
    I: IntoIterator<Item = S>,
    S: Into<String>,
    F: FnMut(&service::ServiceManagerCommand) -> Result<(), CliError>,
{
    run_for_test_with_hooks_and_service_runner(root, interactive, args, |_| Ok(()), runner)
}

pub fn run_for_test_with_hooks_and_service_runner<P, I, S, H, F>(
    root: P,
    interactive: bool,
    args: I,
    daemon_stage: H,
    runner: F,
) -> Result<String, CliError>
where
    P: AsRef<Path>,
    I: IntoIterator<Item = S>,
    S: Into<String>,
    H: FnMut(&AppDirs) -> Result<(), CliError>,
    F: FnMut(&service::ServiceManagerCommand) -> Result<(), CliError>,
{
    let command = Command::from_env(args)?;
    let root = root.as_ref();
    let dirs = AppDirs::for_test(root);
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;

    runtime.block_on(run_test_command(
        root,
        &dirs,
        command,
        interactive,
        daemon_stage,
        runner,
    ))
}

async fn run_test_command<H, F>(
    root: &Path,
    dirs: &AppDirs,
    command: Command,
    interactive: bool,
    daemon_stage: H,
    runner: F,
) -> Result<String, CliError>
where
    H: FnMut(&AppDirs) -> Result<(), CliError>,
    F: FnMut(&service::ServiceManagerCommand) -> Result<(), CliError>,
{
    let mut daemon_stage = daemon_stage;
    match command {
        Command::SmartEntry => {
            let dirs = AppDirs::for_current_user()?;
            run_smart_entry_for_test(&dirs, interactive, &mut daemon_stage)
        }
        Command::Init => {
            let dirs = AppDirs::for_current_user()?;
            run_init_for_test(&dirs, interactive, &mut daemon_stage)
        }
        Command::Status => render_status(dirs),
        Command::Doctor => run_doctor(dirs),
        Command::Service(service_command) => {
            run_service_command_with_runner(dirs, service_command, runner)
        }
        Command::Hooks(hooks_command) => {
            let adapter = resolve_selected_adapter(dirs, || {
                Ok(KimiAdapter::for_test(root.join("kimi-hook.sh")))
            })?;
            run_hooks_command(&adapter, hooks_command)
        }
        Command::Uninstall(mode) => run_uninstall_command(dirs, mode),
        Command::DaemonRun => {
            daemon_run::run_daemon(dirs).await?;
            Ok(String::new())
        }
    }
}

fn run_smart_entry_for_test<H>(
    dirs: &AppDirs,
    interactive: bool,
    daemon_stage: &mut H,
) -> Result<String, CliError>
where
    H: FnMut(&AppDirs) -> Result<(), CliError>,
{
    let metadata = InitMetadata::load(dirs)?;
    if !metadata.initialized && interactive {
        return run_init_for_test(dirs, true, daemon_stage);
    }
    if !metadata.initialized {
        return Ok(SMART_ENTRY_INIT_GUIDANCE.to_string());
    }

    render_status(dirs)
}

fn run_init_for_test<H>(
    dirs: &AppDirs,
    interactive: bool,
    daemon_stage: &mut H,
) -> Result<String, CliError>
where
    H: FnMut(&AppDirs) -> Result<(), CliError>,
{
    run_init_with(dirs, interactive, confirm_install_apply, |plan| {
        apply_init_plan_with_daemon_stage(plan, daemon_stage)
    })
}

fn run_init_with<Confirm, Apply>(
    dirs: &AppDirs,
    interactive: bool,
    mut confirm_apply: Confirm,
    apply_plan: Apply,
) -> Result<String, CliError>
where
    Confirm: FnMut(&InitPlan) -> Result<bool, CliError>,
    Apply: FnOnce(&InitPlan) -> Result<(), CliError>,
{
    if !interactive {
        return Ok(NONINTERACTIVE_INIT_GUIDANCE.to_string());
    }

    let plan = build_current_user_init_plan(InitMode::InteractivePreview, dirs)?;
    let plan_summary = format!("Installation plan\n{}", plan.preview);

    if !confirm_apply(&plan)? {
        return Ok(format!(
            "{plan_summary}\ninstallation cancelled\nRe-run `execmanager init` when you are ready to apply these changes."
        ));
    }

    match apply_plan(&plan) {
        Ok(()) => Ok(format!(
            "{plan_summary}\ninstallation completed\nExecManager is ready."
        )),
        Err(error) => match InitMetadata::load(dirs) {
            Ok(metadata) if !metadata.initialized => Ok(format!(
                "{plan_summary}\ninstallation requires attention\nrecoverable failure: {}\nRun `execmanager doctor` for recovery guidance.",
                error
            )),
            _ => Err(error),
        },
    }
}

fn confirm_install_apply(plan: &InitPlan) -> Result<bool, CliError> {
    if auto_confirm_from_env() {
        return Ok(true);
    }

    print!(
        "Installation plan\n{}\nProceed with installation? [y/N]: ",
        plan.preview
    );
    io::stdout().flush()?;

    let mut response = String::new();
    io::stdin().read_line(&mut response)?;
    let response = response.trim();

    Ok(response.eq_ignore_ascii_case("y") || response.eq_ignore_ascii_case("yes"))
}

fn auto_confirm_from_env() -> bool {
    std::env::var("EXECMANAGER_AUTO_CONFIRM")
        .ok()
        .map(|value| {
            value.eq_ignore_ascii_case("1")
                || value.eq_ignore_ascii_case("true")
                || value.eq_ignore_ascii_case("yes")
                || value.eq_ignore_ascii_case("y")
        })
        .unwrap_or(false)
}

fn run_hooks_command(adapter: &impl Adapter, command: HooksCommand) -> Result<String, CliError> {
    let hook_path = adapter.plan_hook_install().hook_path;

    match command {
        HooksCommand::Install => {
            adapter.install_managed_hook()?;
            Ok(format!("installed managed hook at {}", hook_path.display()))
        }
        HooksCommand::Repair => {
            adapter.repair_managed_hook()?;
            Ok(format!("repaired managed hook at {}", hook_path.display()))
        }
    }
}

fn run_uninstall_command(dirs: &AppDirs, mode: UninstallMode) -> Result<String, CliError> {
    match mode {
        UninstallMode::Safe => {
            run_uninstall(dirs)?;
            Ok(format!(
                "removed execmanager-managed artifacts from {}",
                dirs.config_dir.display()
            ))
        }
        UninstallMode::Restore => run_restore_uninstall(dirs),
    }
}

fn resolve_selected_adapter<T, F>(dirs: &AppDirs, kimi_adapter: F) -> Result<T, CliError>
where
    F: FnOnce() -> Result<T, CliError>,
{
    let metadata = InitMetadata::load(dirs)?;
    let Some(selected_adapter) = metadata.selected_adapter.as_deref() else {
        return Err("hooks command requires a selected adapter in init metadata".into());
    };

    match selected_adapter {
        "kimi" => kimi_adapter(),
        other => Err(format!("hooks command has unsupported adapter: {other}").into()),
    }
}

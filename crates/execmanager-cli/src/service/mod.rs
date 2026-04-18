mod linux;
mod macos;

use std::{
    env,
    path::{Path, PathBuf},
    process::Command as ProcessCommand,
};

use crate::{CliError, app_dirs::AppDirs, commands::ServiceCommand};

const SYSTEMD_UNIT_NAME: &str = "dev.execmanager.daemon.service";
#[cfg(target_os = "macos")]
const LAUNCH_AGENT_LABEL: &str = "dev.execmanager.daemon";
#[cfg(target_os = "macos")]
const LAUNCH_AGENT_FILE_NAME: &str = "dev.execmanager.daemon.plist";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ServiceKind {
    SystemdUser,
    LaunchAgent,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaunchSpec {
    pub execmanager_path: PathBuf,
    pub config_dir: PathBuf,
    pub runtime_dir: PathBuf,
    pub state_dir: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServiceManagerCommand {
    pub program: String,
    pub args: Vec<String>,
}

impl ServiceKind {
    pub fn render(&self, spec: &LaunchSpec) -> String {
        match self {
            Self::SystemdUser => linux::render_unit(spec),
            Self::LaunchAgent => macos::render_launch_agent(spec),
        }
    }

    fn control_commands(
        &self,
        dirs: &AppDirs,
        command: &ServiceCommand,
    ) -> Result<Vec<ServiceManagerCommand>, CliError> {
        match self {
            Self::SystemdUser => {
                let mut commands = Vec::new();

                if matches!(command, ServiceCommand::Start) {
                    commands.push(ServiceManagerCommand {
                        program: "systemctl".to_string(),
                        args: vec!["--user".to_string(), "daemon-reload".to_string()],
                    });
                }

                commands.push(ServiceManagerCommand {
                    program: "systemctl".to_string(),
                    args: vec![
                        "--user".to_string(),
                        service_action(command).to_string(),
                        SYSTEMD_UNIT_NAME.to_string(),
                    ],
                });

                Ok(commands)
            }
            Self::LaunchAgent => {
                #[cfg(target_os = "macos")]
                {
                    let launchctl_target = launchctl_target()?;

                    return Ok(match command {
                        ServiceCommand::Start => vec![ServiceManagerCommand {
                            program: "launchctl".to_string(),
                            args: vec![
                                "bootstrap".to_string(),
                                launchctl_domain()?,
                                managed_service_definition_path(dirs)?.display().to_string(),
                            ],
                        }],
                        ServiceCommand::Stop => vec![ServiceManagerCommand {
                            program: "launchctl".to_string(),
                            args: vec!["bootout".to_string(), launchctl_target],
                        }],
                        ServiceCommand::Restart => vec![ServiceManagerCommand {
                            program: "launchctl".to_string(),
                            args: vec!["kickstart".to_string(), "-k".to_string(), launchctl_target],
                        }],
                    });
                }

                #[cfg(not(target_os = "macos"))]
                {
                    let _ = dirs;
                    let _ = command;
                    Err("LaunchAgent control is only supported on macOS".into())
                }
            }
        }
    }
}

pub fn run_service_command(dirs: &AppDirs, command: ServiceCommand) -> Result<String, CliError> {
    run_service_command_with_runner(dirs, command, real_service_manager_runner)
}

pub(crate) fn current_user_service_definition_path() -> Result<PathBuf, CliError> {
    let home = env::var_os("HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .ok_or("HOME is not set")?;

    Ok(service_definition_path_for_root(&home))
}

pub(crate) fn service_definition_path_for_root(root: &Path) -> PathBuf {
    #[cfg(target_os = "linux")]
    {
        root.join(".config")
            .join("systemd")
            .join("user")
            .join(SYSTEMD_UNIT_NAME)
    }

    #[cfg(target_os = "macos")]
    {
        root.join("Library")
            .join("LaunchAgents")
            .join(LAUNCH_AGENT_FILE_NAME)
    }

    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        root.join(".config")
            .join("systemd")
            .join("user")
            .join(SYSTEMD_UNIT_NAME)
    }
}

pub fn run_service_command_with_runner<F>(
    dirs: &AppDirs,
    command: ServiceCommand,
    mut runner: F,
) -> Result<String, CliError>
where
    F: FnMut(&ServiceManagerCommand) -> Result<(), CliError>,
{
    for service_command in current_service_kind().control_commands(dirs, &command)? {
        runner(&service_command)?;
    }

    Ok(match command {
        ServiceCommand::Start => "started execmanager service".to_string(),
        ServiceCommand::Stop => "stopped execmanager service".to_string(),
        ServiceCommand::Restart => "restarted execmanager service".to_string(),
    })
}

fn real_service_manager_runner(command: &ServiceManagerCommand) -> Result<(), CliError> {
    let status = ProcessCommand::new(&command.program)
        .args(&command.args)
        .status()?;

    if status.success() {
        Ok(())
    } else {
        Err(format!(
            "service manager command failed: {} {}",
            command.program,
            command.args.join(" ")
        )
        .into())
    }
}

fn service_action(command: &ServiceCommand) -> &'static str {
    match command {
        ServiceCommand::Start => "start",
        ServiceCommand::Stop => "stop",
        ServiceCommand::Restart => "restart",
    }
}

fn current_service_kind() -> ServiceKind {
    #[cfg(target_os = "linux")]
    {
        ServiceKind::SystemdUser
    }

    #[cfg(target_os = "macos")]
    {
        ServiceKind::LaunchAgent
    }

    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        ServiceKind::SystemdUser
    }
}

#[cfg(target_os = "macos")]
fn managed_service_definition_path(dirs: &AppDirs) -> Result<PathBuf, CliError> {
    let _ = dirs;
    current_user_service_definition_path()
}

#[cfg(target_os = "macos")]
fn launchctl_domain() -> Result<String, CliError> {
    Ok(format!("gui/{}", current_uid()))
}

#[cfg(target_os = "macos")]
fn launchctl_target() -> Result<String, CliError> {
    Ok(format!("{}/{}", launchctl_domain()?, LAUNCH_AGENT_LABEL))
}

#[cfg(target_os = "macos")]
fn current_uid() -> u32 {
    unsafe extern "C" {
        fn getuid() -> u32;
    }

    unsafe { getuid() }
}

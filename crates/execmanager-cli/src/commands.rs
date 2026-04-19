use std::error::Error;
use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ServiceCommand {
    Start,
    Stop,
    Restart,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HooksCommand {
    Install,
    Repair,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UninstallMode {
    Safe,
    Restore,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    SmartEntry,
    Help,
    Init,
    Status,
    Doctor,
    Service(ServiceCommand),
    Hooks(HooksCommand),
    Uninstall(UninstallMode),
    DaemonRun,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandParseError {
    message: String,
}

impl CommandParseError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl fmt::Display for CommandParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl Error for CommandParseError {}

impl Command {
    pub fn from_env<I, S>(args: I) -> Result<Self, CommandParseError>
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Self::parse_args(args)
    }

    pub fn parse_from<I, S>(args: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Self::parse_args(args).unwrap_or_else(|error| panic!("{error}"))
    }

    fn parse_args<I, S>(args: I) -> Result<Self, CommandParseError>
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let args: Vec<String> = args.into_iter().map(Into::into).collect();
        let tail = args.get(1..).unwrap_or(&[]);

        match tail {
            [] => Ok(Self::SmartEntry),
            [flag] if flag == "-h" || flag == "--help" => Ok(Self::Help),
            [command] if command == "init" => Ok(Self::Init),
            [command] if command == "status" => Ok(Self::Status),
            [command] if command == "doctor" => Ok(Self::Doctor),
            [command] if command == "uninstall" => Ok(Self::Uninstall(UninstallMode::Safe)),
            [command, flag] if command == "uninstall" => {
                Self::parse_uninstall(flag).map(Self::Uninstall)
            }
            [command, action] if command == "service" => {
                Self::parse_service(action).map(Self::Service)
            }
            [command, action] if command == "hooks" => Self::parse_hooks(action).map(Self::Hooks),
            [command, action] if command == "daemon" && action == "run" => Ok(Self::DaemonRun),
            [command, ..] => Err(CommandParseError::new(format!(
                "unknown command: {command}"
            ))),
        }
    }

    fn parse_service(action: &str) -> Result<ServiceCommand, CommandParseError> {
        match action {
            "start" => Ok(ServiceCommand::Start),
            "stop" => Ok(ServiceCommand::Stop),
            "restart" => Ok(ServiceCommand::Restart),
            _ => Err(CommandParseError::new(format!(
                "unknown service command: {action}"
            ))),
        }
    }

    fn parse_hooks(action: &str) -> Result<HooksCommand, CommandParseError> {
        match action {
            "install" => Ok(HooksCommand::Install),
            "repair" => Ok(HooksCommand::Repair),
            _ => Err(CommandParseError::new(format!(
                "unknown hooks command: {action}"
            ))),
        }
    }

    fn parse_uninstall(flag: &str) -> Result<UninstallMode, CommandParseError> {
        match flag {
            "--restore" => Ok(UninstallMode::Restore),
            _ => Err(CommandParseError::new(format!(
                "unknown uninstall flag: {flag}"
            ))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Command, UninstallMode};

    #[test]
    fn parses_top_level_help_flags() {
        assert_eq!(Command::parse_from(["execmanager", "-h"]), Command::Help);
        assert_eq!(
            Command::parse_from(["execmanager", "--help"]),
            Command::Help
        );
    }

    #[test]
    fn parses_safe_uninstall_without_flag() {
        let command = Command::parse_from(["execmanager", "uninstall"]);

        assert_eq!(command, Command::Uninstall(UninstallMode::Safe));
    }

    #[test]
    fn parses_restore_uninstall_with_flag() {
        let command = Command::parse_from(["execmanager", "uninstall", "--restore"]);

        assert_eq!(command, Command::Uninstall(UninstallMode::Restore));
    }
}

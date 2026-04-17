mod kimi;

use std::{fmt, io, path::PathBuf};

pub use kimi::KimiAdapter;

pub trait Adapter {
    fn key(&self) -> &'static str;
    fn detect(&self) -> AdapterDetection;
    fn plan_hook_install(&self) -> HookPlan;
    fn install_managed_hook(&self) -> Result<(), AdapterError>;
    fn repair_managed_hook(&self) -> Result<(), AdapterError>;
    fn uninstall_managed_hook(&self) -> Result<(), AdapterError>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AdapterDetection {
    Detected { hook_path: PathBuf },
    NotDetected { reason: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HookPlan {
    pub hook_path: PathBuf,
    pub managed_region_present: bool,
}

#[derive(Debug)]
pub enum AdapterError {
    Io(io::Error),
    Conflict { message: String },
}

impl AdapterError {
    pub fn conflict(message: impl Into<String>) -> Self {
        Self::Conflict {
            message: message.into(),
        }
    }
}

impl fmt::Display for AdapterError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(f, "adapter IO error: {error}"),
            Self::Conflict { message } => f.write_str(message),
        }
    }
}

impl std::error::Error for AdapterError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            Self::Conflict { .. } => None,
        }
    }
}

impl From<io::Error> for AdapterError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

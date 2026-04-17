use std::fs;
use std::io;

use serde::{Deserialize, Serialize};

use crate::{app_dirs::AppDirs, persist};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum InstallState {
    #[default]
    NotInstalled,
    Installing,
    Installed,
    RepairNeeded,
    FailedPartial,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InitMetadata {
    pub initialized: bool,
    pub selected_adapter: Option<String>,
    pub service_kind: Option<String>,
    #[serde(default)]
    pub install_state: InstallState,
    #[serde(default)]
    pub install_version: Option<String>,
}

impl InitMetadata {
    pub fn load(dirs: &AppDirs) -> io::Result<Self> {
        let path = dirs.metadata_file();

        match fs::read(&path) {
            Ok(bytes) => serde_json::from_slice(&bytes)
                .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error)),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(Self::default()),
            Err(error) => Err(error),
        }
    }

    pub fn store(&self, dirs: &AppDirs) -> io::Result<()> {
        let bytes = serde_json::to_vec_pretty(self)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        persist::atomic_write(&dirs.metadata_file(), &bytes)
    }
}

impl Default for InitMetadata {
    fn default() -> Self {
        Self {
            initialized: false,
            selected_adapter: None,
            service_kind: None,
            install_state: InstallState::NotInstalled,
            install_version: None,
        }
    }
}

use std::{
    fs, io,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};

use crate::{app_dirs::AppDirs, persist};

const RECOVERY_METADATA_FILE_NAME: &str = "recovery.json";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum HookInstallMode {
    NewFile,
    AppendManagedRegion,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HookInstallOutcome {
    pub hook_path: PathBuf,
    pub previous_contents: Option<String>,
    pub install_mode: HookInstallMode,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecoveryMetadata {
    pub selected_adapter: String,
    pub hook_install_mode: HookInstallMode,
    pub hook_backup_contents: Option<String>,
    pub service_definition_path: PathBuf,
    pub service_previously_present: bool,
    #[serde(default)]
    pub service_definition_backup_contents: Option<String>,
    pub fully_restorable: bool,
}

impl RecoveryMetadata {
    pub fn load(dirs: &AppDirs) -> io::Result<Self> {
        let path = recovery_metadata_path(dirs);
        let bytes = fs::read(path)?;

        serde_json::from_slice(&bytes)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
    }

    pub fn store(&self, dirs: &AppDirs) -> io::Result<()> {
        let bytes = serde_json::to_vec_pretty(self)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        persist::atomic_write(&recovery_metadata_path(dirs), &bytes)
    }
}

fn recovery_metadata_path(dirs: &AppDirs) -> PathBuf {
    dirs.config_dir.join(Path::new(RECOVERY_METADATA_FILE_NAME))
}

use std::{
    env, io,
    path::{Path, PathBuf},
};

const APP_DIR_NAME: &str = "execmanager";
const CONFIG_DIR_NAME: &str = "config";
const RUNTIME_DIR_NAME: &str = "runtime";
const STATE_DIR_NAME: &str = "state";
const METADATA_FILE_NAME: &str = "execmanager.json";
const CONFIG_DIR_ENV_NAME: &str = "EXECMANAGER_CONFIG_DIR";
const RUNTIME_DIR_ENV_NAME: &str = "EXECMANAGER_RUNTIME_DIR";
const STATE_DIR_ENV_NAME: &str = "EXECMANAGER_STATE_DIR";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppDirs {
    pub config_dir: PathBuf,
    pub runtime_dir: PathBuf,
    pub state_dir: PathBuf,
}

impl AppDirs {
    pub fn for_current_user() -> io::Result<Self> {
        let home = env::var_os("HOME")
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "HOME is not set"))?;

        Ok(Self::from_home_with_overrides(
            &home,
            env_path_override(CONFIG_DIR_ENV_NAME),
            env_path_override(RUNTIME_DIR_ENV_NAME),
            env_path_override(STATE_DIR_ENV_NAME),
        ))
    }

    pub fn from_home(home: &Path) -> Self {
        #[cfg(target_os = "linux")]
        {
            Self {
                config_dir: home.join(".config").join(APP_DIR_NAME),
                runtime_dir: home.join(".local").join("run").join(APP_DIR_NAME),
                state_dir: home.join(".local").join("state").join(APP_DIR_NAME),
            }
        }

        #[cfg(target_os = "macos")]
        {
            let app_support = home
                .join("Library")
                .join("Application Support")
                .join(APP_DIR_NAME);

            Self {
                config_dir: app_support.clone(),
                runtime_dir: app_support.join("runtime"),
                state_dir: app_support.join("state"),
            }
        }

        #[cfg(not(any(target_os = "linux", target_os = "macos")))]
        {
            Self {
                config_dir: home.join(".config").join(APP_DIR_NAME),
                runtime_dir: home.join(".local").join("run").join(APP_DIR_NAME),
                state_dir: home.join(".local").join("state").join(APP_DIR_NAME),
            }
        }
    }

    pub fn for_test(root: &Path) -> Self {
        Self {
            config_dir: root.join(CONFIG_DIR_NAME),
            runtime_dir: root.join(RUNTIME_DIR_NAME),
            state_dir: root.join(STATE_DIR_NAME),
        }
    }

    pub fn metadata_file(&self) -> PathBuf {
        self.config_dir.join(METADATA_FILE_NAME)
    }

    fn from_home_with_overrides(
        home: &Path,
        config_dir: Option<PathBuf>,
        runtime_dir: Option<PathBuf>,
        state_dir: Option<PathBuf>,
    ) -> Self {
        let defaults = Self::from_home(home);

        Self {
            config_dir: config_dir.unwrap_or(defaults.config_dir),
            runtime_dir: runtime_dir.unwrap_or(defaults.runtime_dir),
            state_dir: state_dir.unwrap_or(defaults.state_dir),
        }
    }
}

fn env_path_override(name: &str) -> Option<PathBuf> {
    env::var_os(name)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

#[cfg(test)]
mod tests {
    use std::{
        ffi::{OsStr, OsString},
        path::PathBuf,
        sync::{Mutex, MutexGuard, OnceLock},
    };

    use tempfile::TempDir;

    use super::AppDirs;

    struct EnvVarGuard {
        name: &'static str,
        original: Option<OsString>,
    }

    impl EnvVarGuard {
        fn set(name: &'static str, value: impl AsRef<OsStr>) -> Self {
            let original = std::env::var_os(name);

            unsafe {
                std::env::set_var(name, value);
            }

            Self { name, original }
        }

        fn remove(name: &'static str) -> Self {
            let original = std::env::var_os(name);

            unsafe {
                std::env::remove_var(name);
            }

            Self { name, original }
        }
    }

    impl Drop for EnvVarGuard {
        fn drop(&mut self) {
            match &self.original {
                Some(value) => unsafe {
                    std::env::set_var(self.name, value);
                },
                None => unsafe {
                    std::env::remove_var(self.name);
                },
            }
        }
    }

    fn env_lock() -> MutexGuard<'static, ()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();

        LOCK.get_or_init(|| Mutex::new(()))
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    struct CurrentUserEnv {
        _root: TempDir,
        _home: EnvVarGuard,
        _config: EnvVarGuard,
        _runtime: EnvVarGuard,
        _state: EnvVarGuard,
        home: PathBuf,
    }

    impl CurrentUserEnv {
        fn with_home_defaults() -> Self {
            let root = tempfile::tempdir().expect("tempdir");
            let home = root.path().join("home");

            Self {
                _home: EnvVarGuard::set("HOME", &home),
                _config: EnvVarGuard::remove("EXECMANAGER_CONFIG_DIR"),
                _runtime: EnvVarGuard::remove("EXECMANAGER_RUNTIME_DIR"),
                _state: EnvVarGuard::remove("EXECMANAGER_STATE_DIR"),
                home,
                _root: root,
            }
        }

        fn with_explicit_overrides() -> (Self, PathBuf, PathBuf, PathBuf) {
            let root = tempfile::tempdir().expect("tempdir");
            let home = root.path().join("home");
            let config_dir = root.path().join("custom-config");
            let runtime_dir = root.path().join("custom-runtime");
            let state_dir = root.path().join("custom-state");

            (
                Self {
                    _home: EnvVarGuard::set("HOME", &home),
                    _config: EnvVarGuard::set("EXECMANAGER_CONFIG_DIR", &config_dir),
                    _runtime: EnvVarGuard::set("EXECMANAGER_RUNTIME_DIR", &runtime_dir),
                    _state: EnvVarGuard::set("EXECMANAGER_STATE_DIR", &state_dir),
                    home,
                    _root: root,
                },
                config_dir,
                runtime_dir,
                state_dir,
            )
        }
    }

    #[test]
    fn prefers_explicit_env_overrides_over_home_defaults() {
        let _lock = env_lock();
        let (_env, config_dir, runtime_dir, state_dir) = CurrentUserEnv::with_explicit_overrides();

        let dirs = AppDirs::for_current_user().expect("dirs");

        assert_eq!(dirs.config_dir, config_dir);
        assert_eq!(dirs.runtime_dir, runtime_dir);
        assert_eq!(dirs.state_dir, state_dir);
    }

    #[test]
    fn falls_back_to_home_defaults_when_overrides_are_absent() {
        let _lock = env_lock();
        let env = CurrentUserEnv::with_home_defaults();

        let dirs = AppDirs::for_current_user().expect("dirs");
        let defaults = AppDirs::from_home(&env.home);

        assert_eq!(dirs.config_dir, defaults.config_dir);
        assert_eq!(dirs.runtime_dir, defaults.runtime_dir);
        assert_eq!(dirs.state_dir, defaults.state_dir);
    }
}

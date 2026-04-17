use execmanager_cli::{
    adapters::{Adapter, KimiAdapter},
    app_dirs::AppDirs,
    recovery::{HookInstallMode, RecoveryMetadata},
    uninstall::{run_restore_uninstall, run_uninstall},
};
use std::{
    ffi::{OsStr, OsString},
    path::Path,
    sync::{Mutex, MutexGuard, OnceLock},
};

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
        .expect("env lock")
}

#[test]
fn safe_uninstall_removes_only_managed_artifacts() {
    let temp = tempfile::tempdir().expect("tempdir");
    let dirs = AppDirs::for_test(temp.path());
    std::fs::create_dir_all(&dirs.config_dir).expect("config dir");
    std::fs::write(dirs.config_dir.join("user-note.txt"), "keep me").expect("write note");
    std::fs::write(dirs.config_dir.join("execmanager.json"), "{}").expect("write metadata");

    run_uninstall(&dirs).expect("safe uninstall");

    assert!(dirs.config_dir.join("user-note.txt").exists());
}

#[test]
fn restore_uninstall_restores_original_hook_contents_when_backup_exists() {
    let temp = tempfile::tempdir().expect("tempdir");
    let dirs = AppDirs::for_test(temp.path());
    let hook_path = temp.path().join("kimi-hook.sh");

    KimiAdapter::for_test(hook_path.clone())
        .install_managed_hook()
        .expect("install managed hook");

    RecoveryMetadata {
        selected_adapter: "kimi".to_string(),
        hook_install_mode: HookInstallMode::AppendManagedRegion,
        hook_backup_contents: Some("#!/bin/sh\n# original hook\n".to_string()),
        service_definition_path: temp.path().join("service-file"),
        service_previously_present: false,
        service_definition_backup_contents: None,
        fully_restorable: true,
    }
    .store(&dirs)
    .expect("store recovery metadata");

    run_restore_uninstall(&dirs).expect("restore uninstall");

    let restored = std::fs::read_to_string(&hook_path).expect("read restored hook");
    assert_eq!(restored, "#!/bin/sh\n# original hook\n");
}

#[test]
fn restore_uninstall_falls_back_to_safe_removal_when_hook_restore_is_impossible() {
    let temp = tempfile::tempdir().expect("tempdir");
    let dirs = AppDirs::for_test(temp.path());
    let hook_path = temp.path().join("kimi-hook.sh");

    KimiAdapter::for_test(hook_path.clone())
        .install_managed_hook()
        .expect("install managed hook");

    RecoveryMetadata {
        selected_adapter: "kimi".to_string(),
        hook_install_mode: HookInstallMode::AppendManagedRegion,
        hook_backup_contents: None,
        service_definition_path: temp.path().join("service-file"),
        service_previously_present: false,
        service_definition_backup_contents: None,
        fully_restorable: false,
    }
    .store(&dirs)
    .expect("store recovery metadata");

    let output = run_restore_uninstall(&dirs).expect("restore uninstall");

    assert!(!hook_path.exists());
    assert!(output.contains("safe removal"));
}

#[test]
fn restore_uninstall_uses_adapter_managed_hook_path_under_config_override() {
    let _lock = env_lock();
    let temp = tempfile::tempdir().expect("tempdir");
    let home = temp.path().join("home");
    let _home = EnvVarGuard::set("HOME", &home);

    let dirs = AppDirs {
        config_dir: temp.path().join("custom-config"),
        runtime_dir: temp.path().join("custom-runtime"),
        state_dir: temp.path().join("custom-state"),
    };
    std::fs::create_dir_all(&dirs.config_dir).expect("config dir");

    let actual_hook_path = home
        .join(".config")
        .join("kimi")
        .join("hooks")
        .join("execmanager-hook.sh");
    let guessed_hook_path = temp
        .path()
        .join("kimi")
        .join("hooks")
        .join("execmanager-hook.sh");

    KimiAdapter::new()
        .expect("adapter")
        .install_managed_hook()
        .expect("install managed hook");

    RecoveryMetadata {
        selected_adapter: "kimi".to_string(),
        hook_install_mode: HookInstallMode::AppendManagedRegion,
        hook_backup_contents: Some("#!/bin/sh\n# restored via adapter path\n".to_string()),
        service_definition_path: temp.path().join("service-file"),
        service_previously_present: false,
        service_definition_backup_contents: None,
        fully_restorable: true,
    }
    .store(&dirs)
    .expect("store recovery metadata");

    run_restore_uninstall(&dirs).expect("restore uninstall");

    assert_eq!(
        std::fs::read_to_string(&actual_hook_path).expect("read actual restored hook"),
        "#!/bin/sh\n# restored via adapter path\n"
    );
    assert!(!guessed_hook_path.exists());
}

#[test]
fn restore_uninstall_accepts_legitimate_current_user_service_path_under_config_override() {
    let _lock = env_lock();
    let temp = tempfile::tempdir().expect("tempdir");
    let home = temp.path().join("home");
    let _home = EnvVarGuard::set("HOME", &home);

    let dirs = AppDirs {
        config_dir: temp.path().join("custom-config"),
        runtime_dir: temp.path().join("custom-runtime"),
        state_dir: temp.path().join("custom-state"),
    };
    std::fs::create_dir_all(&dirs.config_dir).expect("config dir");

    let service_path = managed_service_definition_path(&home);
    std::fs::create_dir_all(service_path.parent().expect("service parent"))
        .expect("service parent dir");
    std::fs::write(&service_path, "old current-user service").expect("write service");

    RecoveryMetadata {
        selected_adapter: "kimi".to_string(),
        hook_install_mode: HookInstallMode::NewFile,
        hook_backup_contents: None,
        service_definition_path: service_path.clone(),
        service_previously_present: true,
        service_definition_backup_contents: Some("restored current-user service".to_string()),
        fully_restorable: true,
    }
    .store(&dirs)
    .expect("store recovery metadata");

    let output = run_restore_uninstall(&dirs).expect("restore uninstall");

    assert_eq!(
        std::fs::read_to_string(&service_path).expect("read restored service"),
        "restored current-user service"
    );
    assert!(!output.contains("did not match the expected managed service path"));
}

#[test]
fn restore_uninstall_does_not_trust_mismatched_service_path_from_metadata() {
    let temp = tempfile::tempdir().expect("tempdir");
    let dirs = AppDirs::for_test(temp.path());
    let expected_service_path = managed_service_definition_path(temp.path());
    let mismatched_service_path = temp.path().join("user-file.service");

    std::fs::create_dir_all(expected_service_path.parent().expect("service parent"))
        .expect("service parent dir");
    std::fs::write(&expected_service_path, "managed service").expect("write expected service");
    std::fs::write(&mismatched_service_path, "user service").expect("write mismatched service");

    RecoveryMetadata {
        selected_adapter: "kimi".to_string(),
        hook_install_mode: HookInstallMode::NewFile,
        hook_backup_contents: None,
        service_definition_path: mismatched_service_path.clone(),
        service_previously_present: true,
        service_definition_backup_contents: Some("restored user service".to_string()),
        fully_restorable: true,
    }
    .store(&dirs)
    .expect("store recovery metadata");

    let output = run_restore_uninstall(&dirs).expect("restore uninstall");

    assert!(!expected_service_path.exists());
    assert_eq!(
        std::fs::read_to_string(&mismatched_service_path).expect("read mismatched service"),
        "user service"
    );
    assert!(output.contains("did not match the expected managed service path"));
}

#[cfg(target_os = "linux")]
fn managed_service_definition_path(root: &Path) -> std::path::PathBuf {
    root.join(".config")
        .join("systemd")
        .join("user")
        .join("dev.execmanager.daemon.service")
}

#[cfg(target_os = "macos")]
fn managed_service_definition_path(root: &Path) -> std::path::PathBuf {
    root.join("Library")
        .join("LaunchAgents")
        .join("dev.execmanager.daemon.plist")
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn managed_service_definition_path(root: &Path) -> std::path::PathBuf {
    root.join(".config")
        .join("systemd")
        .join("user")
        .join("dev.execmanager.daemon.service")
}

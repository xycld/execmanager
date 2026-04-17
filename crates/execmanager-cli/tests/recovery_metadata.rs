use execmanager_cli::{
    app_dirs::AppDirs,
    metadata::{InitMetadata, InstallState},
    recovery::{HookInstallMode, RecoveryMetadata},
};

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

#[test]
fn init_metadata_tracks_installer_state_machine() {
    let temp = tempfile::tempdir().expect("tempdir");
    let dirs = AppDirs::for_test(temp.path());

    let metadata = InitMetadata {
        initialized: false,
        selected_adapter: Some("kimi".to_string()),
        service_kind: Some("systemd --user".to_string()),
        install_state: InstallState::Installing,
        install_version: Some("0.1.0".to_string()),
    };

    metadata.store(&dirs).expect("store metadata");
    let loaded = InitMetadata::load(&dirs).expect("load metadata");

    assert_eq!(loaded.install_state, InstallState::Installing);
    assert_eq!(loaded.install_version.as_deref(), Some("0.1.0"));
}

#[test]
fn init_metadata_loads_legacy_files_with_conservative_defaults() {
    let temp = tempfile::tempdir().expect("tempdir");
    let dirs = AppDirs::for_test(temp.path());
    std::fs::create_dir_all(&dirs.config_dir).expect("config dir");
    std::fs::write(
        dirs.metadata_file(),
        r#"{
  "initialized": true,
  "selected_adapter": "kimi",
  "service_kind": "systemd --user"
}"#,
    )
    .expect("write metadata");

    let loaded = InitMetadata::load(&dirs).expect("load metadata");

    assert!(loaded.initialized);
    assert_eq!(loaded.selected_adapter.as_deref(), Some("kimi"));
    assert_eq!(loaded.service_kind.as_deref(), Some("systemd --user"));
    assert_eq!(loaded.install_state, InstallState::NotInstalled);
    assert_eq!(loaded.install_version, None);
}

#[test]
fn recovery_metadata_round_trips_preinstall_state() {
    let temp = tempfile::tempdir().expect("tempdir");
    let dirs = AppDirs::for_test(temp.path());

    let recovery = RecoveryMetadata {
        selected_adapter: "kimi".to_string(),
        hook_install_mode: HookInstallMode::AppendManagedRegion,
        hook_backup_contents: Some("#!/bin/sh\n# original\n".to_string()),
        service_definition_path: "/tmp/dev.execmanager.daemon.service".into(),
        service_previously_present: false,
        service_definition_backup_contents: Some("[Unit]\nDescription=previous\n".to_string()),
        fully_restorable: true,
    };

    recovery.store(&dirs).expect("store recovery metadata");
    let loaded = RecoveryMetadata::load(&dirs).expect("load recovery metadata");

    assert_eq!(
        loaded.hook_install_mode,
        HookInstallMode::AppendManagedRegion
    );
    assert_eq!(
        loaded.hook_backup_contents.as_deref(),
        Some("#!/bin/sh\n# original\n")
    );
    assert_eq!(
        loaded.service_definition_backup_contents.as_deref(),
        Some("[Unit]\nDescription=previous\n")
    );
}

#[cfg(unix)]
#[test]
fn metadata_store_replaces_read_only_existing_file() {
    let temp = tempfile::tempdir().expect("tempdir");
    let dirs = AppDirs::for_test(temp.path());
    std::fs::create_dir_all(&dirs.config_dir).expect("config dir");
    std::fs::write(dirs.metadata_file(), b"stale").expect("seed metadata");

    let mut permissions = std::fs::metadata(dirs.metadata_file())
        .expect("metadata")
        .permissions();
    permissions.set_mode(0o444);
    std::fs::set_permissions(dirs.metadata_file(), permissions).expect("set readonly");

    let metadata = InitMetadata {
        initialized: true,
        selected_adapter: Some("kimi".to_string()),
        service_kind: Some("systemd --user".to_string()),
        install_state: InstallState::Installed,
        install_version: Some("0.1.0".to_string()),
    };

    metadata.store(&dirs).expect("store metadata");
    let loaded = InitMetadata::load(&dirs).expect("load metadata");

    assert!(loaded.initialized);
    assert_eq!(loaded.install_state, InstallState::Installed);
    assert_eq!(loaded.install_version.as_deref(), Some("0.1.0"));
}

#[cfg(unix)]
#[test]
fn recovery_store_replaces_read_only_existing_file() {
    let temp = tempfile::tempdir().expect("tempdir");
    let dirs = AppDirs::for_test(temp.path());
    std::fs::create_dir_all(&dirs.config_dir).expect("config dir");
    std::fs::write(dirs.config_dir.join("recovery.json"), b"stale").expect("seed recovery");

    let recovery_path = dirs.config_dir.join("recovery.json");
    let mut permissions = std::fs::metadata(&recovery_path)
        .expect("metadata")
        .permissions();
    permissions.set_mode(0o444);
    std::fs::set_permissions(&recovery_path, permissions).expect("set readonly");

    let recovery = RecoveryMetadata {
        selected_adapter: "kimi".to_string(),
        hook_install_mode: HookInstallMode::AppendManagedRegion,
        hook_backup_contents: Some("#!/bin/sh\n# original\n".to_string()),
        service_definition_path: "/tmp/dev.execmanager.daemon.service".into(),
        service_previously_present: false,
        service_definition_backup_contents: Some("[Unit]\nDescription=previous\n".to_string()),
        fully_restorable: true,
    };

    recovery.store(&dirs).expect("store recovery metadata");
    let loaded = RecoveryMetadata::load(&dirs).expect("load recovery metadata");

    assert_eq!(loaded.selected_adapter, "kimi");
    assert_eq!(
        loaded.hook_install_mode,
        HookInstallMode::AppendManagedRegion
    );
}

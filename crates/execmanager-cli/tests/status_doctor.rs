use execmanager_cli::{
    app_dirs::AppDirs, doctor::run_doctor, init::verify::verify_daemon_readiness,
    metadata::{InitMetadata, InstallState},
    recovery::{HookInstallMode, RecoveryMetadata},
    status::render_status,
};
use execmanager_daemon::{spawn_rpc_server, DaemonRpcConfig};

#[test]
fn render_status_reports_initialized_adapter_service_and_paths() {
    let temp = tempfile::tempdir().expect("tempdir");
    let dirs = AppDirs::for_test(temp.path());
    InitMetadata {
        initialized: true,
        selected_adapter: Some("kimi".to_string()),
        service_kind: Some("systemd --user".to_string()),
        install_state: InstallState::Installed,
        install_version: None,
    }
    .store(&dirs)
    .expect("store metadata");

    let rendered = render_status(&dirs).expect("render status");

    assert!(rendered.contains("initialized: yes"));
    assert!(rendered.contains("adapter: kimi"));
    assert!(rendered.contains("service: systemd --user"));
    assert!(rendered.contains(&format!("config dir: {}", dirs.config_dir.display())));
    assert!(rendered.contains(&format!("runtime dir: {}", dirs.runtime_dir.display())));
    assert!(rendered.contains(&format!("state dir: {}", dirs.state_dir.display())));
}

#[test]
fn doctor_reports_missing_socket_with_remediation_hint() {
    let temp = tempfile::tempdir().expect("tempdir");
    let dirs = AppDirs::for_test(temp.path());
    std::fs::create_dir_all(&dirs.config_dir).expect("config dir");
    InitMetadata {
        initialized: true,
        selected_adapter: Some("kimi".to_string()),
        service_kind: Some("systemd --user".to_string()),
        install_state: InstallState::Installed,
        install_version: None,
    }
    .store(&dirs)
    .expect("store metadata");

    let doctor = run_doctor(&dirs).expect("run doctor");

    assert!(doctor.contains("daemon not reachable"));
    assert!(doctor.contains("execmanager init"));
    assert!(doctor.contains("execmanager daemon run"));
    assert!(!doctor.contains("execmanager service start"));
}

#[test]
fn status_reports_failed_partial_install_state_and_version() {
    let temp = tempfile::tempdir().expect("tempdir");
    let dirs = AppDirs::for_test(temp.path());
    InitMetadata {
        initialized: false,
        selected_adapter: Some("kimi".to_string()),
        service_kind: Some("systemd --user".to_string()),
        install_state: InstallState::FailedPartial,
        install_version: Some("0.1.0".to_string()),
    }
    .store(&dirs)
    .expect("store metadata");

    let rendered = render_status(&dirs).expect("status");

    assert!(rendered.contains("install state: failed-partial"));
    assert!(rendered.contains("install version: 0.1.0"));
}

#[test]
fn doctor_reports_partial_install_restore_and_recovery_next_steps() {
    let temp = tempfile::tempdir().expect("tempdir");
    let dirs = AppDirs::for_test(temp.path());
    std::fs::create_dir_all(&dirs.config_dir).expect("config dir");
    InitMetadata {
        initialized: false,
        selected_adapter: Some("kimi".to_string()),
        service_kind: Some("systemd --user".to_string()),
        install_state: InstallState::FailedPartial,
        install_version: Some("0.1.0".to_string()),
    }
    .store(&dirs)
    .expect("store metadata");
    RecoveryMetadata {
        selected_adapter: "kimi".to_string(),
        hook_install_mode: HookInstallMode::AppendManagedRegion,
        hook_backup_contents: Some("#!/bin/sh\n# original\n".to_string()),
        service_definition_path: temp.path().join("dev.execmanager.daemon.service"),
        service_previously_present: false,
        service_definition_backup_contents: None,
        fully_restorable: true,
    }
    .store(&dirs)
    .expect("store recovery metadata");

    let doctor = run_doctor(&dirs).expect("run doctor");

    assert!(doctor.contains("install state: failed-partial"));
    assert!(doctor.contains("restore available: yes"));
    assert!(doctor.contains("execmanager uninstall --restore"));
    assert!(doctor.contains("execmanager init"));
    assert!(!doctor.contains("init metadata missing"));
    assert!(!doctor.contains("daemon not reachable"));
}

#[test]
fn verify_daemon_readiness_accepts_live_socket() {
    let temp = tempfile::tempdir().expect("tempdir");
    let dirs = AppDirs::for_test(temp.path());
    std::fs::create_dir_all(&dirs.runtime_dir).expect("runtime dir");
    let socket_path = dirs.runtime_dir.join("execmanager.sock");
    std::fs::create_dir_all(&dirs.state_dir).expect("state dir");
    let journal_path = dirs.state_dir.join("events.journal");
    let runtime = tokio::runtime::Runtime::new().expect("runtime");
    let server = runtime
        .block_on(async { spawn_rpc_server(DaemonRpcConfig::new(&socket_path, &journal_path)) })
        .expect("spawn daemon server");

    verify_daemon_readiness(&dirs).expect("daemon should be reachable");

    runtime
        .block_on(server.shutdown())
        .expect("shutdown daemon server");
}

use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

use execmanager_cli::{
    app_dirs::AppDirs,
    commands::ServiceCommand,
    init::{
        apply_init_plan, apply_init_plan_with_daemon_stage, build_init_plan,
        verify::start_service_and_verify_daemon_readiness_with, InitMode,
    },
    metadata::{InitMetadata, InstallState},
    recovery::{HookInstallMode, RecoveryMetadata},
    service::run_service_command_with_runner,
};

#[cfg(target_os = "linux")]
fn service_definition_path(root: &std::path::Path) -> std::path::PathBuf {
    root.join(".config")
        .join("systemd")
        .join("user")
        .join("dev.execmanager.daemon.service")
}

#[cfg(target_os = "macos")]
fn service_definition_path(root: &std::path::Path) -> std::path::PathBuf {
    root.join("Library")
        .join("LaunchAgents")
        .join("dev.execmanager.daemon.plist")
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn service_definition_path(root: &std::path::Path) -> std::path::PathBuf {
    root.join(".config")
        .join("systemd")
        .join("user")
        .join("dev.execmanager.daemon.service")
}

#[test]
fn apply_init_plan_moves_install_state_to_installed_on_success() {
    let temp = tempfile::tempdir().expect("tempdir");
    let dirs = AppDirs::for_test(temp.path());

    let plan = build_init_plan(InitMode::InteractivePreview, temp.path()).expect("build plan");
    apply_init_plan_with_daemon_stage(&plan, |_| Ok(())).expect("apply plan");

    let metadata = InitMetadata::load(&dirs).expect("load metadata");
    assert_eq!(metadata.install_state, InstallState::Installed);
    assert!(metadata.initialized);
}

#[test]
fn apply_init_plan_persists_installing_before_runtime_dir_side_effects() {
    let temp = tempfile::tempdir().expect("tempdir");
    let dirs = AppDirs::for_test(temp.path());

    std::fs::create_dir_all(temp.path().join(".config").join("kimi").join("hooks"))
        .expect("seed kimi hooks dir");
    std::fs::create_dir_all(&dirs.config_dir).expect("config dir");
    std::fs::write(&dirs.runtime_dir, b"not a directory").expect("seed runtime path as file");

    let plan = build_init_plan(InitMode::InteractivePreview, temp.path()).expect("build plan");
    let error = apply_init_plan(&plan).expect_err("apply should fail before runtime dir creation");
    assert!(!error.to_string().is_empty());

    let metadata = InitMetadata::load(&dirs).expect("load metadata");
    assert_eq!(metadata.install_state, InstallState::Installing);
    assert!(!metadata.initialized);
    assert!(!temp.path().join("kimi-hook.sh").exists());
}

#[test]
fn failed_apply_leaves_recoverable_failed_partial_state_without_overstating_restore() {
    let temp = tempfile::tempdir().expect("tempdir");
    let dirs = AppDirs::for_test(temp.path());
    let service_definition_path = service_definition_path(temp.path());

    std::fs::create_dir_all(&service_definition_path).expect("seed service path as directory");

    let plan = build_init_plan(InitMode::InteractivePreview, temp.path()).expect("build plan");

    let error = apply_init_plan(&plan).expect_err("apply should fail");
    assert!(!error.to_string().is_empty());

    let metadata = InitMetadata::load(&dirs).expect("load metadata");
    assert_eq!(metadata.install_state, InstallState::FailedPartial);
    assert!(!metadata.initialized);
    assert_eq!(metadata.selected_adapter.as_deref(), Some("kimi"));

    let recovery = RecoveryMetadata::load(&dirs).expect("load recovery metadata");
    assert_eq!(recovery.selected_adapter, "kimi");
    assert_eq!(recovery.hook_install_mode, HookInstallMode::NewFile);
    assert_eq!(recovery.service_definition_path, service_definition_path);
    assert!(recovery.service_previously_present);
    assert_eq!(recovery.service_definition_backup_contents, None);
    assert!(!recovery.fully_restorable);
}

#[test]
fn recovery_metadata_captures_previous_service_definition_contents_when_overwriting_file() {
    let temp = tempfile::tempdir().expect("tempdir");
    let dirs = AppDirs::for_test(temp.path());
    let service_definition_path = service_definition_path(temp.path());

    std::fs::create_dir_all(
        service_definition_path
            .parent()
            .expect("service definition parent"),
    )
    .expect("service parent dir");
    std::fs::write(&service_definition_path, "[Unit]\nDescription=previous\n")
        .expect("seed previous service definition");

    let plan = build_init_plan(InitMode::InteractivePreview, temp.path()).expect("build plan");
    apply_init_plan_with_daemon_stage(&plan, |_| Ok(())).expect("apply plan");

    let recovery = RecoveryMetadata::load(&dirs).expect("load recovery metadata");
    assert!(recovery.service_previously_present);
    assert_eq!(
        recovery.service_definition_backup_contents.as_deref(),
        Some("[Unit]\nDescription=previous\n")
    );
    assert!(recovery.fully_restorable);
}

#[test]
fn successful_install_verifies_daemon_readiness_before_committing() {
    let temp = tempfile::tempdir().expect("tempdir");
    let dirs = AppDirs::for_test(temp.path());
    let daemon_stage_invoked = Rc::new(Cell::new(false));
    let readiness_attempts = Rc::new(Cell::new(0));
    let service_commands = Rc::new(RefCell::new(Vec::new()));

    let plan = build_init_plan(InitMode::InteractivePreview, temp.path()).expect("build plan");
    let daemon_stage_invoked_for_apply = daemon_stage_invoked.clone();
    let readiness_attempts_for_apply = readiness_attempts.clone();
    let service_commands_for_apply = service_commands.clone();
    let result = apply_init_plan_with_daemon_stage(&plan, |daemon_dirs| {
        assert_eq!(daemon_dirs.runtime_dir, dirs.runtime_dir);
        assert!(service_definition_path(temp.path()).exists());
        daemon_stage_invoked_for_apply.set(true);

        start_service_and_verify_daemon_readiness_with(
            daemon_dirs,
            |service_dirs| {
                run_service_command_with_runner(service_dirs, ServiceCommand::Start, |command| {
                    service_commands_for_apply.borrow_mut().push(format!(
                        "{} {}",
                        command.program,
                        command.args.join(" ")
                    ));
                    Ok(())
                })?;
                Ok(())
            },
            |verify_dirs| {
                assert_eq!(verify_dirs.runtime_dir, dirs.runtime_dir);
                let next_attempt = readiness_attempts_for_apply.get() + 1;
                readiness_attempts_for_apply.set(next_attempt);

                if next_attempt < 3 {
                    Err(format!("readiness attempt {next_attempt} not ready yet").into())
                } else {
                    Ok(())
                }
            },
        )
    });

    assert!(result.is_ok());
    assert!(daemon_stage_invoked.get());
    assert_eq!(readiness_attempts.get(), 3);

    #[cfg(target_os = "linux")]
    assert_eq!(
        service_commands.borrow().as_slice(),
        &[
            "systemctl --user daemon-reload".to_string(),
            "systemctl --user start dev.execmanager.daemon.service".to_string(),
        ]
    );

    #[cfg(target_os = "macos")]
    assert_eq!(service_commands.borrow().len(), 1);

    let metadata = InitMetadata::load(&dirs).expect("load metadata");
    assert_eq!(metadata.install_state, InstallState::Installed);
    assert!(metadata.initialized);
}

#[test]
fn failed_daemon_readiness_keeps_install_state_failed_partial() {
    let temp = tempfile::tempdir().expect("tempdir");
    let dirs = AppDirs::for_test(temp.path());
    let readiness_attempts = Rc::new(Cell::new(0));

    let plan = build_init_plan(InitMode::InteractivePreview, temp.path()).expect("build plan");
    let readiness_attempts_for_apply = readiness_attempts.clone();
    let error = apply_init_plan_with_daemon_stage(&plan, |daemon_dirs| {
        start_service_and_verify_daemon_readiness_with(
            daemon_dirs,
            |service_dirs| {
                run_service_command_with_runner(service_dirs, ServiceCommand::Start, |_| Ok(()))?;
                Ok(())
            },
            |_| {
                let next_attempt = readiness_attempts_for_apply.get() + 1;
                readiness_attempts_for_apply.set(next_attempt);
                Err("daemon socket is still missing".into())
            },
        )
    })
    .expect_err("daemon readiness failure should fail install");

    assert!(error
        .to_string()
        .contains("timed out waiting for daemon readiness"));
    assert!(readiness_attempts.get() > 1);

    let metadata = InitMetadata::load(&dirs).expect("load metadata");
    assert_eq!(metadata.install_state, InstallState::FailedPartial);
    assert!(!metadata.initialized);
    assert!(service_definition_path(temp.path()).exists());

    let recovery = RecoveryMetadata::load(&dirs).expect("load recovery metadata");
    assert_eq!(recovery.selected_adapter, "kimi");
}

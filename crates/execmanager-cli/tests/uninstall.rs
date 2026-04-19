use std::path::{Path, PathBuf};

use execmanager_cli::{
    app_dirs::AppDirs,
    init::{apply_init_plan_with_daemon_stage, build_init_plan, InitMode},
    uninstall::run_uninstall_with_service_runner,
};

#[test]
fn uninstall_removes_managed_artifacts_but_not_unrelated_user_files() {
    let temp = tempfile::tempdir().expect("tempdir");
    let root = temp.path();
    let dirs = AppDirs::for_test(root);
    std::fs::create_dir_all(&dirs.config_dir).expect("config dir");
    std::fs::write(dirs.config_dir.join("user-note.txt"), "keep me").expect("write note");

    let plan = build_init_plan(InitMode::InteractivePreview, root).expect("build plan");
    apply_init_plan_with_daemon_stage(&plan, |_| Ok(())).expect("apply plan");

    let service_definition_path = managed_service_definition_path(root);
    let hook_path = root.join("kimi-hook.sh");
    std::fs::write(dirs.runtime_dir.join("execmanager.sock"), "socket").expect("write socket");
    std::fs::write(dirs.state_dir.join("events.journal"), "journal").expect("write journal");

    assert!(dirs.metadata_file().exists());
    assert!(hook_path.exists());
    assert!(service_definition_path.exists());

    let service_commands = std::cell::RefCell::new(Vec::<String>::new());

    run_uninstall_with_service_runner(&dirs, |command| {
        service_commands.borrow_mut().push(format!(
            "{} {}",
            command.program,
            command.args.join(" ")
        ));
        Ok(())
    })
    .expect("uninstall");

    assert!(dirs.config_dir.join("user-note.txt").exists());
    assert!(!dirs.metadata_file().exists());
    assert!(!hook_path.exists());
    assert!(!service_definition_path.exists());
    assert!(!dirs.runtime_dir.join("execmanager.sock").exists());
    assert!(!dirs.state_dir.join("events.journal").exists());

    #[cfg(target_os = "linux")]
    assert_eq!(
        service_commands.borrow().as_slice(),
        &[
            "systemctl --user stop dev.execmanager.daemon.service".to_string(),
            "systemctl --user daemon-reload".to_string(),
        ]
    );
}

#[cfg(target_os = "linux")]
fn managed_service_definition_path(root: &Path) -> PathBuf {
    root.join(".config/systemd/user/dev.execmanager.daemon.service")
}

#[cfg(target_os = "macos")]
fn managed_service_definition_path(root: &Path) -> PathBuf {
    root.join("Library/LaunchAgents/dev.execmanager.daemon.plist")
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn managed_service_definition_path(root: &Path) -> PathBuf {
    root.join(".config/systemd/user/dev.execmanager.daemon.service")
}

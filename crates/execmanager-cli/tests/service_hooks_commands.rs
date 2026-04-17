use execmanager_cli::{
    app_dirs::AppDirs,
    metadata::{InitMetadata, InstallState},
    run_for_test, run_for_test_with_service_runner,
    service::ServiceManagerCommand,
};

#[cfg(target_os = "macos")]
fn current_uid() -> u32 {
    unsafe extern "C" {
        fn getuid() -> u32;
    }

    unsafe { getuid() }
}

#[test]
fn hooks_install_requires_selected_adapter_metadata() {
    let temp = tempfile::tempdir().expect("tempdir");

    let error = run_for_test(temp.path(), false, ["execmanager", "hooks", "install"])
        .expect_err("hooks install should require selected adapter metadata");

    assert!(error.to_string().contains("selected adapter"));
}

#[test]
fn hooks_install_dispatch_installs_selected_kimi_hook_in_test_root() {
    let temp = tempfile::tempdir().expect("tempdir");
    InitMetadata {
        initialized: true,
        selected_adapter: Some("kimi".to_string()),
        service_kind: Some("systemd --user".to_string()),
        install_state: InstallState::Installed,
        install_version: None,
    }
    .store(&AppDirs::for_test(temp.path()))
    .expect("store metadata");

    let output = run_for_test(temp.path(), false, ["execmanager", "hooks", "install"])
        .expect("hooks install output");

    let hook_path = temp.path().join("kimi-hook.sh");
    let contents = std::fs::read_to_string(&hook_path).expect("read hook");

    assert_eq!(
        output,
        format!("installed managed hook at {}", hook_path.display())
    );
    assert!(contents.contains("BEGIN EXECMANAGER MANAGED HOOK"));
}

#[test]
fn hooks_repair_dispatch_repairs_existing_hook_in_test_root() {
    let temp = tempfile::tempdir().expect("tempdir");
    InitMetadata {
        initialized: true,
        selected_adapter: Some("kimi".to_string()),
        service_kind: Some("systemd --user".to_string()),
        install_state: InstallState::Installed,
        install_version: None,
    }
    .store(&AppDirs::for_test(temp.path()))
    .expect("store metadata");
    let hook_path = temp.path().join("kimi-hook.sh");
    std::fs::write(&hook_path, "#!/bin/sh\n# existing prelude\n").expect("write existing hook");

    let output = run_for_test(temp.path(), false, ["execmanager", "hooks", "repair"])
        .expect("hooks repair output");

    let contents = std::fs::read_to_string(&hook_path).expect("read hook");

    assert_eq!(
        output,
        format!("repaired managed hook at {}", hook_path.display())
    );
    assert!(contents.starts_with("#!/bin/sh\n# existing prelude\n"));
    assert!(contents.contains("BEGIN EXECMANAGER MANAGED HOOK"));
}

#[test]
fn hooks_install_rejects_unsupported_selected_adapter() {
    let temp = tempfile::tempdir().expect("tempdir");
    InitMetadata {
        initialized: true,
        selected_adapter: Some("other".to_string()),
        service_kind: Some("systemd --user".to_string()),
        install_state: InstallState::Installed,
        install_version: None,
    }
    .store(&AppDirs::for_test(temp.path()))
    .expect("store metadata");

    let error = run_for_test(temp.path(), false, ["execmanager", "hooks", "install"])
        .expect_err("hooks install should reject unsupported adapter");

    assert!(error.to_string().contains("unsupported adapter: other"));
}

#[test]
fn service_start_stop_and_restart_dispatch_to_platform_runner() {
    let temp = tempfile::tempdir().expect("tempdir");
    let mut recorded = Vec::new();

    let start_output = run_for_test_with_service_runner(
        temp.path(),
        false,
        ["execmanager", "service", "start"],
        |command: &ServiceManagerCommand| {
            recorded.push(command.clone());
            Ok(())
        },
    )
    .expect("service start output");
    let stop_output = run_for_test_with_service_runner(
        temp.path(),
        false,
        ["execmanager", "service", "stop"],
        |command: &ServiceManagerCommand| {
            recorded.push(command.clone());
            Ok(())
        },
    )
    .expect("service stop output");
    let restart_output = run_for_test_with_service_runner(
        temp.path(),
        false,
        ["execmanager", "service", "restart"],
        |command: &ServiceManagerCommand| {
            recorded.push(command.clone());
            Ok(())
        },
    )
    .expect("service restart output");

    assert_eq!(start_output, "started execmanager service");
    assert_eq!(stop_output, "stopped execmanager service");
    assert_eq!(restart_output, "restarted execmanager service");

    #[cfg(target_os = "linux")]
    assert_eq!(
        recorded,
        vec![
            ServiceManagerCommand {
                program: "systemctl".to_string(),
                args: vec!["--user".to_string(), "daemon-reload".to_string()],
            },
            ServiceManagerCommand {
                program: "systemctl".to_string(),
                args: vec![
                    "--user".to_string(),
                    "start".to_string(),
                    "dev.execmanager.daemon.service".to_string(),
                ],
            },
            ServiceManagerCommand {
                program: "systemctl".to_string(),
                args: vec![
                    "--user".to_string(),
                    "stop".to_string(),
                    "dev.execmanager.daemon.service".to_string(),
                ],
            },
            ServiceManagerCommand {
                program: "systemctl".to_string(),
                args: vec![
                    "--user".to_string(),
                    "restart".to_string(),
                    "dev.execmanager.daemon.service".to_string(),
                ],
            },
        ]
    );

    #[cfg(target_os = "macos")]
    assert_eq!(
        recorded,
        vec![
            ServiceManagerCommand {
                program: "launchctl".to_string(),
                args: vec![
                    "bootstrap".to_string(),
                    format!("gui/{}", current_uid()),
                    AppDirs::for_test(temp.path())
                        .config_dir
                        .parent()
                        .expect("config parent")
                        .parent()
                        .expect("app support parent")
                        .join("LaunchAgents/dev.execmanager.daemon.plist")
                        .display()
                        .to_string(),
                ],
            },
            ServiceManagerCommand {
                program: "launchctl".to_string(),
                args: vec![
                    "bootout".to_string(),
                    format!("gui/{}/dev.execmanager.daemon", current_uid()),
                ],
            },
            ServiceManagerCommand {
                program: "launchctl".to_string(),
                args: vec![
                    "kickstart".to_string(),
                    "-k".to_string(),
                    format!("gui/{}/dev.execmanager.daemon", current_uid()),
                ],
            },
        ]
    );
}

use execmanager_cli::adapters::{Adapter, KimiAdapter};
use execmanager_cli::recovery::{HookInstallMode, HookInstallOutcome};

#[test]
fn install_appends_managed_region_without_destroying_existing_hook_content() {
    let temp = tempfile::tempdir().expect("tempdir");
    let hook_path = temp.path().join("kimi-hook.sh");
    std::fs::write(&hook_path, "#!/bin/sh\n# original hook\n").expect("write hook");

    let adapter = KimiAdapter::for_test(hook_path.clone());
    adapter.install_managed_hook().expect("install");

    let contents = std::fs::read_to_string(&hook_path).expect("read hook");
    assert!(contents.starts_with("#!/bin/sh\n# original hook\n"));
    assert!(contents.contains("BEGIN EXECMANAGER MANAGED HOOK"));
}

#[test]
fn install_reports_conflict_when_existing_managed_region_was_modified() {
    let temp = tempfile::tempdir().expect("tempdir");
    let hook_path = temp.path().join("kimi-hook.sh");
    std::fs::write(
        &hook_path,
        "#!/bin/sh\n# original hook\n# BEGIN EXECMANAGER MANAGED HOOK\n# modified\n# END EXECMANAGER MANAGED HOOK\n",
    )
    .expect("write conflicting hook");

    let adapter = KimiAdapter::for_test(hook_path);
    let error = adapter
        .install_managed_hook()
        .expect_err("install should fail closed");
    assert!(error.to_string().contains("manual resolution"));
}

#[test]
fn install_reports_append_mode_and_previous_contents_for_existing_hook() {
    let temp = tempfile::tempdir().expect("tempdir");
    let hook_path = temp.path().join("kimi-hook.sh");
    std::fs::write(&hook_path, "#!/bin/sh\n# original hook\n").expect("write hook");

    let adapter = KimiAdapter::for_test(hook_path.clone());
    let outcome = adapter
        .install_managed_hook_with_outcome()
        .expect("install with outcome");

    assert_eq!(
        outcome,
        HookInstallOutcome {
            hook_path,
            previous_contents: Some("#!/bin/sh\n# original hook\n".to_string()),
            install_mode: HookInstallMode::AppendManagedRegion,
        }
    );
}

#[test]
fn install_reports_new_file_mode_when_hook_was_missing() {
    let temp = tempfile::tempdir().expect("tempdir");
    let hook_path = temp.path().join("kimi-hook.sh");

    let adapter = KimiAdapter::for_test(hook_path.clone());
    let outcome = adapter
        .install_managed_hook_with_outcome()
        .expect("install with outcome");

    assert_eq!(
        outcome,
        HookInstallOutcome {
            hook_path,
            previous_contents: None,
            install_mode: HookInstallMode::NewFile,
        }
    );
}

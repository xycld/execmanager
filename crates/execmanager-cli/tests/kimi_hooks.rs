use execmanager_cli::adapters::{Adapter, KimiAdapter};

#[test]
fn kimi_hook_install_is_idempotent_and_marked() {
    let temp = tempfile::tempdir().expect("tempdir");
    let hook_path = temp.path().join("kimi-hook.sh");
    let adapter = KimiAdapter::for_test(hook_path.clone());

    adapter.install_managed_hook().expect("first install");
    adapter.install_managed_hook().expect("second install");

    let contents = std::fs::read_to_string(hook_path).expect("read hook");
    assert!(contents.contains("BEGIN EXECMANAGER MANAGED HOOK"));
    assert_eq!(
        contents.matches("BEGIN EXECMANAGER MANAGED HOOK").count(),
        1
    );
}

#[test]
fn kimi_hook_repair_stops_on_user_modified_managed_region() {
    let temp = tempfile::tempdir().expect("tempdir");
    let hook_path = temp.path().join("kimi-hook.sh");
    let adapter = KimiAdapter::for_test(hook_path.clone());

    adapter.install_managed_hook().expect("install hook");
    std::fs::write(
        &hook_path,
        "# BEGIN EXECMANAGER MANAGED HOOK\n# user changed this\n# END EXECMANAGER MANAGED HOOK\n",
    )
    .expect("overwrite hook");

    let error = adapter
        .repair_managed_hook()
        .expect_err("repair should stop on conflict");
    assert!(error.to_string().contains("manual resolution"));
}

#[test]
fn kimi_hook_uninstall_stops_on_user_modified_managed_region() {
    let temp = tempfile::tempdir().expect("tempdir");
    let hook_path = temp.path().join("kimi-hook.sh");
    let adapter = KimiAdapter::for_test(hook_path.clone());

    adapter.install_managed_hook().expect("install hook");
    std::fs::write(
        &hook_path,
        "# BEGIN EXECMANAGER MANAGED HOOK\n# user changed this\n# END EXECMANAGER MANAGED HOOK\n",
    )
    .expect("overwrite hook");

    let error = adapter
        .uninstall_managed_hook()
        .expect_err("uninstall should stop on conflict");
    assert!(error.to_string().contains("manual resolution"));

    let contents = std::fs::read_to_string(hook_path).expect("read preserved hook");
    assert!(contents.contains("# user changed this"));
}

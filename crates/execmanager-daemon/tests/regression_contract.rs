use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("workspace root")
        .to_path_buf()
}

#[test]
fn task12_regression_contract_is_explicit() {
    let root = workspace_root();
    let cargo_config = root.join(".cargo/config.toml");
    let cargo_config_contents = fs::read_to_string(&cargo_config)
        .unwrap_or_else(|_| panic!("expected {} to exist", cargo_config.display()));

    assert!(
        cargo_config_contents
            .contains("verify = \"test --workspace --all-targets -- --nocapture\""),
        "expected {} to define the canonical workspace verification alias",
        cargo_config.display()
    );

    let output = Command::new("cargo")
        .args(["test", "--workspace", "--all-targets", "--", "--list"])
        .current_dir(&root)
        .output()
        .expect("list workspace tests");

    assert!(
        output.status.success(),
        "expected cargo test --workspace --all-targets -- --list to succeed, stderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );

    let listed = String::from_utf8_lossy(&output.stdout);
    for required in [
        "supported_exec_routes_through_daemon",
        "unsupported_exec_is_marked_non_coverage",
        "daemon_version_mismatch_fails_managed_mode",
        "replay_rebuilds_runtime_state",
        "corrupt_event_is_rejected_safely",
        "direct_rm_is_rewritten_to_safe_delete",
        "protected_path_rm_is_blocked",
        "dev_server_becomes_managed_service_with_port_mapping",
        "actual_listener_is_observed_after_bind",
        "unknown_process_is_not_cleaned_up",
        "restart_reconciliation_recovers_managed_state",
        "unknown_execution_cleanup_is_refused",
        "degraded_capability_is_explicit",
        "observable_only_capability_is_explicit_when_setrlimit_is_unavailable",
        "tui_displays_managed_service_lifecycle",
        "degraded_state_is_visible",
        "managed_dev_server_appears_and_opens_viewer",
        "viewer_open_does_not_emit_spawn",
        "rm_safety_rewrite_is_visible_in_tui_history",
    ] {
        assert!(
            listed.contains(required),
            "expected workspace regression contract to include {required:?}; available tests:\n{listed}"
        );
    }
}

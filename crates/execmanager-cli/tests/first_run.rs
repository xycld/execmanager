use execmanager_cli::{
    app_dirs::AppDirs,
    commands::{Command, ServiceCommand},
    init::{apply_init_plan_with_daemon_stage, build_init_plan, InitMode},
    metadata::InitMetadata,
};
use std::{
    ffi::{OsStr, OsString},
    path::{Path, PathBuf},
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
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

struct CurrentUserTestEnv {
    _home: EnvVarGuard,
    _config: EnvVarGuard,
    _runtime: EnvVarGuard,
    _state: EnvVarGuard,
    _auto_confirm: Option<EnvVarGuard>,
    dirs: AppDirs,
    hook_path: PathBuf,
}

fn configure_current_user_test_env(
    root: &Path,
    auto_confirm: Option<impl AsRef<OsStr>>,
    create_hooks_dir: bool,
) -> CurrentUserTestEnv {
    let home = root.join("home");
    let kimi_hooks_dir = home.join(".config").join("kimi").join("hooks");
    if create_hooks_dir {
        std::fs::create_dir_all(&kimi_hooks_dir).expect("create kimi hooks dir");
    }

    let config_dir = root.join("current-user-config");
    let runtime_dir = root.join("current-user-runtime");
    let state_dir = root.join("current-user-state");

    let home_guard = EnvVarGuard::set("HOME", &home);
    let config_guard = EnvVarGuard::set("EXECMANAGER_CONFIG_DIR", &config_dir);
    let runtime_guard = EnvVarGuard::set("EXECMANAGER_RUNTIME_DIR", &runtime_dir);
    let state_guard = EnvVarGuard::set("EXECMANAGER_STATE_DIR", &state_dir);
    let auto_confirm_guard = auto_confirm
        .as_ref()
        .map(|value| EnvVarGuard::set("EXECMANAGER_AUTO_CONFIRM", value));

    let dirs = AppDirs::for_current_user().expect("current user dirs");

    CurrentUserTestEnv {
        _home: home_guard,
        _config: config_guard,
        _runtime: runtime_guard,
        _state: state_guard,
        _auto_confirm: auto_confirm_guard,
        dirs,
        hook_path: home
            .join(".config")
            .join("kimi")
            .join("hooks")
            .join("execmanager-hook.sh"),
    }
}

#[test]
fn parses_public_commands_and_hidden_daemon_run() {
    assert_eq!(Command::parse_from(["execmanager", "init"]), Command::Init);
    assert_eq!(
        Command::parse_from(["execmanager", "status"]),
        Command::Status
    );
    assert_eq!(
        Command::parse_from(["execmanager", "service", "restart"]),
        Command::Service(ServiceCommand::Restart)
    );
    assert_eq!(
        Command::parse_from(["execmanager", "daemon", "run"]),
        Command::DaemonRun
    );
}

#[test]
fn reports_uninitialized_when_metadata_file_is_missing() {
    let temp = tempfile::tempdir().expect("tempdir");
    let dirs = AppDirs::for_test(temp.path());
    let state = InitMetadata::load(&dirs).expect("load state");

    assert!(!state.initialized);
}

#[test]
fn init_plan_previews_hook_service_and_paths_before_apply() {
    let temp = tempfile::tempdir().expect("tempdir");
    let plan = build_init_plan(InitMode::InteractivePreview, temp.path()).expect("build plan");

    assert!(plan.preview.contains("adapter: kimi"));
    assert!(plan.preview.contains("service:"));
    assert!(plan.preview.contains("runtime dir:"));
    assert!(plan.preview.contains("state dir:"));
}

#[test]
fn apply_init_plan_uses_the_reviewed_plan_context() {
    let planned_root = tempfile::tempdir().expect("planned tempdir");
    let different_root = tempfile::tempdir().expect("different tempdir");
    let plan =
        build_init_plan(InitMode::InteractivePreview, planned_root.path()).expect("build plan");

    assert!(!different_root.path().join("kimi-hook.sh").exists());

    apply_init_plan_with_daemon_stage(&plan, |_| Ok(())).expect("apply plan");

    let planned_dirs = AppDirs::for_test(planned_root.path());
    let metadata = InitMetadata::load(&planned_dirs).expect("load metadata");

    assert!(metadata.initialized);
    assert_eq!(metadata.selected_adapter.as_deref(), Some("kimi"));
    assert_eq!(
        metadata.service_kind.as_deref(),
        Some(plan.service_kind.as_str())
    );
    assert!(planned_root.path().join("kimi-hook.sh").exists());
    assert!(!different_root.path().join("kimi-hook.sh").exists());
}

#[test]
fn apply_init_plan_persists_metadata_from_reviewed_context_not_mutated_public_fields() {
    let planned_root = tempfile::tempdir().expect("planned tempdir");
    let mut plan =
        build_init_plan(InitMode::InteractivePreview, planned_root.path()).expect("build plan");

    plan.adapter_key = "mutated-adapter".to_string();
    plan.service_kind = "mutated-service".to_string();

    apply_init_plan_with_daemon_stage(&plan, |_| Ok(())).expect("apply plan");

    let metadata = InitMetadata::load(&AppDirs::for_test(planned_root.path())).expect("metadata");

    assert!(metadata.initialized);
    assert_eq!(metadata.selected_adapter.as_deref(), Some("kimi"));
    assert_ne!(
        metadata.selected_adapter.as_deref(),
        Some("mutated-adapter")
    );
    assert_ne!(metadata.service_kind.as_deref(), Some("mutated-service"));
}

#[test]
fn smart_entry_interactive_path_executes_install_flow_after_confirmation() {
    let _lock = env_lock();
    let temp = tempfile::tempdir().expect("tempdir");
    let env = configure_current_user_test_env(temp.path(), Some("yes"), true);
    let output =
        execmanager_cli::run_for_test(temp.path(), true, ["execmanager"]).expect("run smart entry");

    assert!(output.contains("installation completed"));
    assert!(output.contains(&format!("config dir: {}", env.dirs.config_dir.display())));
    assert!(output.contains(&format!("runtime dir: {}", env.dirs.runtime_dir.display())));
    assert!(output.contains(&format!("state dir: {}", env.dirs.state_dir.display())));
    assert!(output.contains(&format!("hook path: {}", env.hook_path.display())));
    assert!(env.hook_path.exists());

    let metadata = InitMetadata::load(&env.dirs).expect("load metadata");
    assert!(metadata.initialized);
    assert_eq!(metadata.selected_adapter.as_deref(), Some("kimi"));
}

#[test]
fn init_command_executes_install_flow_after_confirmation() {
    let _lock = env_lock();
    let temp = tempfile::tempdir().expect("tempdir");
    let env = configure_current_user_test_env(temp.path(), Some("yes"), true);

    let output = execmanager_cli::run_for_test(temp.path(), true, ["execmanager", "init"])
        .expect("run init command");

    assert!(output.contains("installation completed"));
    assert!(output.contains(&format!("hook path: {}", env.hook_path.display())));

    let metadata = InitMetadata::load(&env.dirs).expect("load metadata");
    assert!(metadata.initialized);
}

#[test]
fn smart_entry_prints_init_guidance_when_non_interactive_and_uninitialized() {
    let _lock = env_lock();
    let temp = tempfile::tempdir().expect("tempdir");
    let _env = configure_current_user_test_env(temp.path(), Some("yes"), true);
    let output = execmanager_cli::run_for_test(temp.path(), false, ["execmanager"])
        .expect("run smart entry");

    assert_eq!(
        output,
        "ExecManager is not initialized. Run `execmanager init`."
    );
}

#[test]
fn init_command_requires_interactive_terminal() {
    let _lock = env_lock();
    let temp = tempfile::tempdir().expect("tempdir");
    let _env = configure_current_user_test_env(temp.path(), Some("yes"), true);

    let output = execmanager_cli::run_for_test(temp.path(), false, ["execmanager", "init"])
        .expect("run init command");

    assert_eq!(
        output,
        "ExecManager init requires an interactive terminal. Re-run `execmanager init` from an interactive terminal to review and apply installation changes."
    );
}

#[test]
fn smart_entry_launches_tui_when_initialized_and_interactive() {
    let _lock = env_lock();
    let temp = tempfile::tempdir().expect("tempdir");
    let env = configure_current_user_test_env(temp.path(), Some("yes"), true);
    std::fs::create_dir_all(&env.dirs.state_dir).expect("create state dir");
    std::fs::write(env.dirs.state_dir.join("events.journal"), b"").expect("seed empty journal");
    InitMetadata {
        initialized: true,
        selected_adapter: Some("kimi".to_string()),
        service_kind: Some("systemd --user".to_string()),
        install_state: execmanager_cli::metadata::InstallState::Installed,
        install_version: None,
    }
    .store(&env.dirs)
    .expect("store metadata");

    let output =
        execmanager_cli::run_for_test(temp.path(), true, ["execmanager"]).expect("run smart entry");

    assert!(output.contains("Instances"));
    assert!(output.contains("Services"));
    assert!(output.contains("History"));
    assert!(output.contains("Ghosts/Reconcile"));
    assert!(output.contains("Selection Summary / Recent State"));
    assert!(output.contains("Recent stdout / stderr"));
}

#[test]
fn smart_entry_prints_operational_summary_when_initialized_and_non_interactive() {
    let _lock = env_lock();
    let temp = tempfile::tempdir().expect("tempdir");
    let env = configure_current_user_test_env(temp.path(), Some("yes"), true);
    InitMetadata {
        initialized: true,
        selected_adapter: Some("kimi".to_string()),
        service_kind: Some("systemd --user".to_string()),
        install_state: execmanager_cli::metadata::InstallState::Installed,
        install_version: None,
    }
    .store(&env.dirs)
    .expect("store metadata");

    let output = execmanager_cli::run_for_test(temp.path(), false, ["execmanager"])
        .expect("run smart entry");

    assert!(output.contains("initialized: yes"));
    assert!(output.contains("adapter: kimi"));
    assert!(output.contains("service: systemd --user"));
}

#[test]
fn init_command_uses_auto_confirm_env_to_reach_real_confirmation_path() {
    let _lock = env_lock();
    let temp = tempfile::tempdir().expect("tempdir");
    let env = configure_current_user_test_env(temp.path(), Some("true"), true);

    let output = execmanager_cli::run_for_test(temp.path(), true, ["execmanager", "init"])
        .expect("run init command");

    assert!(output.contains("installation completed"));
    assert!(env.hook_path.exists());
}

#[test]
fn init_command_does_not_install_when_auto_confirm_env_is_falsey() {
    let _lock = env_lock();
    let temp = tempfile::tempdir().expect("tempdir");
    let env = configure_current_user_test_env(temp.path(), Some("no"), true);

    let output = execmanager_cli::run_for_test(temp.path(), true, ["execmanager", "init"])
        .expect("run init command");

    assert!(output.contains("installation cancelled"));
    assert!(!env.hook_path.exists());

    let metadata = InitMetadata::load(&env.dirs).expect("load metadata");
    assert!(!metadata.initialized);
}

#[test]
fn init_command_installs_even_when_kimi_hooks_dir_is_missing() {
    let _lock = env_lock();
    let temp = tempfile::tempdir().expect("tempdir");
    let env = configure_current_user_test_env(temp.path(), Some("yes"), false);

    assert!(!env.hook_path.parent().expect("hook parent").exists());

    let output = execmanager_cli::run_for_test(temp.path(), true, ["execmanager", "init"])
        .expect("run init command");

    assert!(output.contains("installation completed"));
    assert!(env.hook_path.exists());

    let metadata = InitMetadata::load(&env.dirs).expect("load metadata");
    assert!(metadata.initialized);
    assert_eq!(metadata.selected_adapter.as_deref(), Some("kimi"));
}

#[test]
fn init_command_reports_failed_partial_when_daemon_readiness_fails() {
    let _lock = env_lock();
    let temp = tempfile::tempdir().expect("tempdir");
    let env = configure_current_user_test_env(temp.path(), Some("yes"), false);

    let output = execmanager_cli::run_for_test_with_hooks_and_service_runner(
        temp.path(),
        true,
        ["execmanager", "init"],
        |_| Err("daemon socket is still missing".into()),
        |_| Ok(()),
    )
    .expect("run init command");

    assert!(output.contains("installation requires attention"));
    assert!(output.contains("recoverable failure"));

    let metadata = InitMetadata::load(&env.dirs).expect("load metadata");
    assert!(!metadata.initialized);
    assert_eq!(
        metadata.install_state,
        execmanager_cli::metadata::InstallState::FailedPartial
    );
}

#[cfg(target_os = "linux")]
#[test]
fn derives_stable_user_categories_from_home_on_linux() {
    let home = Path::new("/tmp/execmanager-home");
    let dirs = AppDirs::from_home(home);

    assert_eq!(dirs.config_dir, home.join(".config/execmanager"));
    assert_eq!(dirs.runtime_dir, home.join(".local/run/execmanager"));
    assert_eq!(dirs.state_dir, home.join(".local/state/execmanager"));
}

#[cfg(target_os = "macos")]
#[test]
fn derives_stable_user_categories_from_home_on_macos() {
    let home = Path::new("/tmp/execmanager-home");
    let dirs = AppDirs::from_home(home);

    assert_eq!(
        dirs.config_dir,
        home.join("Library/Application Support/execmanager")
    );
    assert_eq!(
        dirs.runtime_dir,
        home.join("Library/Application Support/execmanager/runtime")
    );
    assert_eq!(
        dirs.state_dir,
        home.join("Library/Application Support/execmanager/state")
    );
}

use execmanager_cli::service::{LaunchSpec, ServiceKind};

#[test]
fn renders_systemd_user_unit_for_same_binary() {
    let rendered = ServiceKind::SystemdUser.render(&LaunchSpec {
        execmanager_path: "/tmp/execmanager".into(),
        config_dir: "/tmp/config".into(),
        runtime_dir: "/tmp/runtime".into(),
        state_dir: "/tmp/state".into(),
    });

    assert!(rendered.contains("ExecStart=/tmp/execmanager daemon run"));
}

#[test]
fn renders_launch_agent_for_same_binary() {
    let rendered = ServiceKind::LaunchAgent.render(&LaunchSpec {
        execmanager_path: "/tmp/execmanager".into(),
        config_dir: "/tmp/config".into(),
        runtime_dir: "/tmp/runtime".into(),
        state_dir: "/tmp/state".into(),
    });

    assert!(rendered.contains("daemon"));
    assert!(rendered.contains("run"));
}

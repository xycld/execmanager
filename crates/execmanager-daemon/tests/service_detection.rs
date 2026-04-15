use std::net::TcpListener;
use std::path::PathBuf;
use std::thread;
use std::time::Duration;

use execmanager_contracts::{ExecutionId, ProjectionState};
use execmanager_daemon::{ExecutionMode, ManagedExecutor, ManagedLaunchSpec, RuntimeProjection};
use tempfile::tempdir;

fn python3_program() -> PathBuf {
    PathBuf::from("/usr/bin/python3")
}

fn reserve_local_port() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").expect("reserve local port");
    let port = listener.local_addr().expect("local addr").port();
    drop(listener);
    port
}

fn delayed_listener_program(port: u16, bind_delay_ms: u64, lifetime_ms: u64) -> Vec<String> {
    vec![
        "-c".to_string(),
        format!(
            r#"import socket, time; time.sleep({bind_delay}); sock = socket.socket(); sock.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1); sock.bind(('127.0.0.1', {port})); sock.listen(8); time.sleep({lifetime})"#,
            bind_delay = bind_delay_ms as f64 / 1000.0,
            port = port,
            lifetime = lifetime_ms as f64 / 1000.0,
        ),
    ]
}

#[test]
fn dev_server_becomes_managed_service_with_port_mapping() {
    let temp = tempdir().expect("tempdir");
    let journal_path = temp.path().join("events.journal");
    let port = reserve_local_port();
    let mut executor = ManagedExecutor::new(&journal_path).expect("create managed executor");
    let exec_id = ExecutionId::new("exec-service-detect-001");

    let managed = executor
        .launch(
            ManagedLaunchSpec::new(
                exec_id.clone(),
                python3_program(),
                delayed_listener_program(port, 0, 2_000),
                ExecutionMode::BatchPipes,
            )
            .with_original_command(format!("python3 delayed-listener {port}")),
        )
        .expect("listener process should launch");

    let observation = managed
        .observe_runtime_facts(Duration::from_secs(2))
        .expect("listener facts should be observed");

    assert_eq!(observation.listeners.len(), 1);
    assert_eq!(observation.listeners[0].port, port);
    assert_eq!(observation.service_name, "observed_service");

    executor
        .override_service_classification(&exec_id, ProjectionState::ShortTask)
        .expect("service override should be recorded without mutating observed truth");

    let projection = RuntimeProjection::replay_from_path(&journal_path).expect("replay journal");
    let execution = projection
        .execution(exec_id.as_str())
        .expect("execution projection exists");
    assert_eq!(execution.observed_state, ProjectionState::Service);
    assert_eq!(execution.state, ProjectionState::ShortTask);
    assert_eq!(execution.service_override, Some(ProjectionState::ShortTask));

    let service = projection
        .service(exec_id.as_str(), "observed_service")
        .expect("service projection exists");
    assert_eq!(service.state, ProjectionState::Service);
    assert_eq!(service.port_ids, vec![format!("tcp:{port}")]);

    let observed_port = projection
        .port(exec_id.as_str(), &format!("tcp:{port}"))
        .expect("port projection exists");
    assert_eq!(observed_port.port, port);
    assert_eq!(observed_port.protocol, "tcp");

    let viewer_handle = projection
        .viewer_handle(&exec_id)
        .expect("viewer handle should come from daemon-owned projection");
    assert_eq!(viewer_handle.exec_id.as_str(), exec_id.as_str());
}

#[test]
fn actual_listener_is_observed_after_bind() {
    let temp = tempdir().expect("tempdir");
    let journal_path = temp.path().join("events.journal");
    let port = reserve_local_port();
    let mut executor = ManagedExecutor::new(&journal_path).expect("create managed executor");
    let exec_id = ExecutionId::new("exec-service-detect-002");

    let managed = executor
        .launch(
            ManagedLaunchSpec::new(
                exec_id.clone(),
                python3_program(),
                delayed_listener_program(port, 400, 2_000),
                ExecutionMode::BatchPipes,
            )
            .with_original_command(format!("python3 delayed-listener {port}")),
        )
        .expect("listener process should launch");

    let projection_before_bind =
        RuntimeProjection::replay_from_path(&journal_path).expect("replay journal");
    assert!(
        projection_before_bind
            .port(exec_id.as_str(), &format!("tcp:{port}"))
            .is_none(),
        "daemon must not guess a future listener before bind"
    );

    thread::sleep(Duration::from_millis(700));

    let observation = managed
        .observe_runtime_facts(Duration::from_secs(2))
        .expect("actual listener should be observed after bind");
    assert_eq!(observation.listeners.len(), 1);
    assert_eq!(observation.listeners[0].port, port);

    let projection_after_bind =
        RuntimeProjection::replay_from_path(&journal_path).expect("replay journal");
    assert_eq!(
        projection_after_bind.port(exec_id.as_str(), &format!("tcp:{port}")),
        Some(&execmanager_daemon::PortView {
            port_id: format!("tcp:{port}"),
            port,
            protocol: "tcp".to_string(),
            state: ProjectionState::Service,
        })
    );
}

#[test]
fn service_override_preserves_observed_runtime_truth() {
    let temp = tempdir().expect("tempdir");
    let journal_path = temp.path().join("events.journal");
    let port = reserve_local_port();
    let mut executor = ManagedExecutor::new(&journal_path).expect("create managed executor");
    let exec_id = ExecutionId::new("exec-service-detect-003");

    let managed = executor
        .launch(
            ManagedLaunchSpec::new(
                exec_id.clone(),
                python3_program(),
                delayed_listener_program(port, 0, 1_500),
                ExecutionMode::BatchPipes,
            )
            .with_original_command(format!("python3 delayed-listener {port}")),
        )
        .expect("listener process should launch");

    managed
        .observe_runtime_facts(Duration::from_secs(2))
        .expect("runtime facts should classify the exec as a service");

    executor
        .override_service_classification(&exec_id, ProjectionState::Managed)
        .expect("override should be stored as annotation");

    let projection = RuntimeProjection::replay_from_path(&journal_path).expect("replay journal");
    let execution = projection
        .execution(exec_id.as_str())
        .expect("execution projection exists");

    assert_eq!(execution.observed_state, ProjectionState::Service);
    assert_eq!(execution.service_override, Some(ProjectionState::Managed));
    assert_eq!(execution.state, ProjectionState::Managed);
}

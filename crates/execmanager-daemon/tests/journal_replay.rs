use std::fs::OpenOptions;
use std::io::{Seek, SeekFrom, Write};

use execmanager_contracts::{ExecutionId, ProjectionState};
use execmanager_daemon::{
    BlobReference, CorruptionKind, GhostView, Journal, JournalEvent, ReplayError,
    RuntimeProjection, ServiceView,
};
use tempfile::tempdir;

#[test]
fn replay_rebuilds_runtime_state() {
    let temp = tempdir().expect("tempdir");
    let journal_path = temp.path().join("events.journal");

    let mut journal = Journal::open(&journal_path).expect("open journal");
    let exec_id = ExecutionId::new("exec-replay-001");
    let stdout_blob = BlobReference {
        blob_id: "blob-stdout-001".to_string(),
        sha256: "abc123def456".to_string(),
        size_bytes: 4096,
        media_type: "text/plain".to_string(),
        storage_path: "blobs/stdout/blob-stdout-001".to_string(),
    };

    let events = vec![
        JournalEvent::ExecutionRegistered {
            exec_id: exec_id.clone(),
            state: ProjectionState::Managed,
            command: "python -m http.server".to_string(),
            stdout: Some(stdout_blob.clone()),
            stderr: None,
        },
        JournalEvent::ServiceObserved {
            exec_id: exec_id.clone(),
            service: ServiceView {
                name: "demo-http".to_string(),
                state: ProjectionState::Service,
                port_ids: vec!["port-http-001".to_string()],
            },
        },
        JournalEvent::PortObserved {
            exec_id: exec_id.clone(),
            port: execmanager_daemon::PortView {
                port_id: "port-http-001".to_string(),
                port: 8000,
                protocol: "tcp".to_string(),
                state: ProjectionState::Service,
            },
        },
        JournalEvent::GhostStateRecorded {
            exec_id: exec_id.clone(),
            ghost: GhostView {
                state: ProjectionState::Detached,
                detail: "socket remained after service stop".to_string(),
            },
        },
        JournalEvent::ExecutionStateUpdated {
            exec_id: exec_id.clone(),
            state: ProjectionState::Exited,
        },
    ];

    for event in events {
        journal.append(&event).expect("append event");
    }

    let rebuilt = RuntimeProjection::replay_from_path(&journal_path).expect("replay journal");

    let execution = rebuilt
        .execution(exec_id.as_str())
        .expect("execution projection exists");
    assert_eq!(execution.exec_id.as_str(), exec_id.as_str());
    assert_eq!(execution.state, ProjectionState::Exited);
    assert_eq!(execution.command, "python -m http.server");
    assert_eq!(execution.stdout, Some(stdout_blob));
    assert_eq!(execution.stderr, None);

    assert_eq!(
        rebuilt.service(exec_id.as_str(), "demo-http"),
        Some(&ServiceView {
            name: "demo-http".to_string(),
            state: ProjectionState::Service,
            port_ids: vec!["port-http-001".to_string()],
        })
    );

    assert_eq!(
        rebuilt.port(exec_id.as_str(), "port-http-001"),
        Some(&execmanager_daemon::PortView {
            port_id: "port-http-001".to_string(),
            port: 8000,
            protocol: "tcp".to_string(),
            state: ProjectionState::Service,
        })
    );

    assert_eq!(
        rebuilt.ghost(exec_id.as_str()),
        Some(&GhostView {
            state: ProjectionState::Detached,
            detail: "socket remained after service stop".to_string(),
        })
    );
    assert!(!rebuilt.is_degraded());
}

#[test]
fn corrupt_event_is_rejected_safely() {
    let temp = tempdir().expect("tempdir");
    let journal_path = temp.path().join("events.journal");

    let mut journal = Journal::open(&journal_path).expect("open journal");
    let exec_id = ExecutionId::new("exec-corrupt-001");
    journal
        .append(&JournalEvent::ExecutionRegistered {
            exec_id: exec_id.clone(),
            state: ProjectionState::Managed,
            command: "sleep 10".to_string(),
            stdout: None,
            stderr: None,
        })
        .expect("append good event");
    journal
        .append(&JournalEvent::ExecutionStateUpdated {
            exec_id: exec_id.clone(),
            state: ProjectionState::Exited,
        })
        .expect("append second event");

    let mut file = OpenOptions::new()
        .read(true)
        .write(true)
        .open(&journal_path)
        .expect("open file for corruption");
    file.seek(SeekFrom::End(-1)).expect("seek to file tail");
    file.write_all(&[0xAA]).expect("overwrite tail byte");
    file.flush().expect("flush corruption");

    let error = RuntimeProjection::replay_from_path(&journal_path)
        .expect_err("corrupt tail record must be rejected safely");

    match error {
        ReplayError::CorruptRecord {
            offset,
            kind: CorruptionKind::ChecksumMismatch,
        } => {
            assert!(offset > 0);
        }
        other => panic!("expected checksum mismatch, got {other:?}"),
    }

    let degraded = RuntimeProjection::replay_with_degraded_state(&journal_path)
        .expect("degraded replay still returns state wrapper");
    assert!(degraded.is_degraded());
    assert_eq!(degraded.degraded_reason(), Some(&error));
    assert!(degraded.execution(exec_id.as_str()).is_some());
}

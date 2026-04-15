use execmanager_contracts::{ExecutionId, ViewerRequest};
use execmanager_daemon::{
    BlobReference, ExecutionMode, ExecutionView, RuntimeOwnership, ViewerHandle,
};
use execmanager_viewers::{
    attach_viewer, ViewerAdapter, ViewerAttachError, ViewerAttachment, ViewerOwnership,
};

#[derive(Default)]
struct RecordingAdapter {
    attached_exec_ids: Vec<String>,
}

impl ViewerAdapter for RecordingAdapter {
    fn attach(&mut self, handle: &ViewerHandle) -> Result<ViewerAttachment, ViewerAttachError> {
        self.attached_exec_ids
            .push(handle.exec_id.as_str().to_string());
        Ok(ViewerAttachment {
            exec_id: handle.exec_id.clone(),
            ownership: ViewerOwnership::AttachedReadOnly,
        })
    }
}

#[test]
fn viewer_attachment_requires_daemon_handle_and_never_assumes_execution_ownership() {
    let request = ViewerRequest {
        exec_id: ExecutionId::new("exec-viewer-001"),
    };
    let execution = ExecutionView {
        exec_id: request.exec_id.clone(),
        state: execmanager_contracts::ProjectionState::Service,
        observed_state: execmanager_contracts::ProjectionState::Service,
        command: "python3 -m http.server".to_string(),
        original_command: "python3 -m http.server".to_string(),
        rewritten_command: None,
        policy_outcome: None,
        stdout: Some(BlobReference {
            blob_id: "blob-stdout-001".to_string(),
            sha256: "abc123".to_string(),
            size_bytes: 16,
            media_type: "text/plain".to_string(),
            storage_path: "blobs/stdout/blob-stdout-001".to_string(),
        }),
        stderr: None,
        mode: Some(ExecutionMode::BatchPipes),
        lifecycle: vec![],
        ownership: Some(RuntimeOwnership {
            root_pid: 4242,
            process_group_id: 4242,
            session_id: Some(4242),
            start_time_ticks: Some(99),
        }),
        resource_governance: None,
        service_override: None,
    };
    let handle = ViewerHandle::from_execution(&execution, None).expect("handle from daemon state");

    let mut adapter = RecordingAdapter::default();
    let attachment = attach_viewer(&request, &handle, &mut adapter).expect("viewer should attach");

    assert_eq!(attachment.exec_id.as_str(), "exec-viewer-001");
    assert_eq!(attachment.ownership, ViewerOwnership::AttachedReadOnly);
    assert_eq!(
        adapter.attached_exec_ids,
        vec!["exec-viewer-001".to_string()]
    );
}

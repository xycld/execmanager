use execmanager_contracts::{ExecutionId, ViewerRequest};
use execmanager_daemon::ViewerHandle;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ViewerAttachment {
    pub exec_id: ExecutionId,
    pub ownership: ViewerOwnership,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ViewerOwnership {
    AttachedReadOnly,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ViewerAttachError {
    RequestHandleMismatch {
        requested_exec_id: ExecutionId,
        handle_exec_id: ExecutionId,
    },
}

impl std::fmt::Display for ViewerAttachError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::RequestHandleMismatch {
                requested_exec_id,
                handle_exec_id,
            } => write!(
                f,
                "viewer request {} does not match daemon handle {}",
                requested_exec_id.as_str(),
                handle_exec_id.as_str()
            ),
        }
    }
}

impl std::error::Error for ViewerAttachError {}

pub trait ViewerAdapter {
    fn attach(&mut self, handle: &ViewerHandle) -> Result<ViewerAttachment, ViewerAttachError>;
}

pub fn attach_viewer(
    request: &ViewerRequest,
    handle: &ViewerHandle,
    adapter: &mut impl ViewerAdapter,
) -> Result<ViewerAttachment, ViewerAttachError> {
    if request.exec_id != handle.exec_id {
        return Err(ViewerAttachError::RequestHandleMismatch {
            requested_exec_id: request.exec_id.clone(),
            handle_exec_id: handle.exec_id.clone(),
        });
    }

    adapter.attach(handle)
}

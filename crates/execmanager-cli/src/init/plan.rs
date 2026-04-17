use std::path::Path;

use crate::{
    adapters::Adapter,
    app_dirs::AppDirs,
    init::detect::{detect_for_current_user, detect_for_test_root, InitContext},
    recovery::RecoveryMetadata,
    CliError,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InitMode {
    InteractivePreview,
    Apply,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InitPlan {
    pub adapter_key: String,
    pub service_kind: String,
    pub preview: String,
    pub should_start_daemon: bool,
    pub(crate) context: InitContext,
}

impl InitPlan {
    pub(crate) fn recovery_metadata_for(
        &self,
        hook_install_mode: crate::recovery::HookInstallMode,
        hook_backup_contents: Option<String>,
    ) -> RecoveryMetadata {
        let fully_restorable = !self.context.service_previously_present
            || self.context.service_definition_backup_contents.is_some();

        RecoveryMetadata {
            selected_adapter: self.context.adapter_key.clone(),
            hook_install_mode,
            hook_backup_contents,
            service_definition_path: self.context.service_definition_path.clone(),
            service_previously_present: self.context.service_previously_present,
            service_definition_backup_contents: self
                .context
                .service_definition_backup_contents
                .clone(),
            fully_restorable,
        }
    }
}

pub fn build_init_plan(mode: InitMode, root: &Path) -> Result<InitPlan, CliError> {
    build_plan(mode, detect_for_test_root(root)?)
}

pub fn build_current_user_init_plan(mode: InitMode, dirs: &AppDirs) -> Result<InitPlan, CliError> {
    let execmanager_path = std::env::current_exe()?;
    build_plan(
        mode,
        detect_for_current_user(dirs.clone(), execmanager_path)?,
    )
}

fn build_plan(mode: InitMode, context: InitContext) -> Result<InitPlan, CliError> {
    let hook_plan = context.adapter.plan_hook_install();

    Ok(InitPlan {
        adapter_key: context.adapter_key.clone(),
        service_kind: context.service_label.clone(),
        preview: render_preview(
            mode,
            &context.service_label,
            &context.dirs,
            &hook_plan.hook_path,
            &context.service_definition_path,
        ),
        should_start_daemon: false,
        context,
    })
}

fn render_preview(
    mode: InitMode,
    service_label: &str,
    dirs: &AppDirs,
    hook_path: &Path,
    service_definition_path: &Path,
) -> String {
    let mode_label = match mode {
        InitMode::InteractivePreview => "interactive-preview",
        InitMode::Apply => "apply",
    };

    format!(
        concat!(
            "mode: {}\n",
            "adapter: kimi\n",
            "service: {}\n",
            "config dir: {}\n",
            "runtime dir: {}\n",
            "state dir: {}\n",
            "hook path: {}\n",
            "service definition: {}\n",
            "metadata file: {}\n",
            "daemon start: deferred\n"
        ),
        mode_label,
        service_label,
        dirs.config_dir.display(),
        dirs.runtime_dir.display(),
        dirs.state_dir.display(),
        hook_path.display(),
        service_definition_path.display(),
        dirs.metadata_file().display(),
    )
}

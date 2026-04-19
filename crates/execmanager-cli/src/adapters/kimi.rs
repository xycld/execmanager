use std::{env, fs, io, path::PathBuf};

use super::{Adapter, AdapterDetection, AdapterError, HookPlan};
use crate::recovery::{HookInstallMode, HookInstallOutcome};

const BEGIN_MARKER: &str = "# BEGIN EXECMANAGER MANAGED HOOK";
const END_MARKER: &str = "# END EXECMANAGER MANAGED HOOK";
const VERSION_LINE: &str = "# execmanager-managed-hook-version: 1";
const INVOCATION_LINE: &str = "execmanager daemon run";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KimiAdapter {
    hook_path: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum HookSnapshot {
    NoFile,
    ExistingNonManaged { contents: String },
    MatchingManaged { contents: String },
    ConflictingManaged,
}

impl KimiAdapter {
    pub fn new() -> Result<Self, AdapterError> {
        let hook_path = env::var_os("HOME")
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "HOME is not set"))?
            .join(".config")
            .join("kimi")
            .join("hooks")
            .join("execmanager-hook.sh");

        Ok(Self { hook_path })
    }

    pub fn for_test(hook_path: PathBuf) -> Self {
        Self { hook_path }
    }

    fn managed_region(&self) -> String {
        format!("{BEGIN_MARKER}\n{VERSION_LINE}\n{INVOCATION_LINE}\n{END_MARKER}\n")
    }

    fn read_existing_hook(&self) -> Result<Option<String>, AdapterError> {
        match fs::read_to_string(&self.hook_path) {
            Ok(contents) => Ok(Some(contents)),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error.into()),
        }
    }

    fn snapshot_existing_hook(&self) -> Result<HookSnapshot, AdapterError> {
        let Some(contents) = self.read_existing_hook()? else {
            return Ok(HookSnapshot::NoFile);
        };

        let managed_region = self.managed_region();

        match Self::marker_bounds(&contents) {
            None => Ok(HookSnapshot::ExistingNonManaged { contents }),
            Some((start, end)) => {
                let current_region = &contents[start..end];
                if current_region == managed_region {
                    Ok(HookSnapshot::MatchingManaged { contents })
                } else {
                    Ok(HookSnapshot::ConflictingManaged)
                }
            }
        }
    }

    fn write_hook(&self, contents: &str) -> Result<(), AdapterError> {
        if let Some(parent) = self.hook_path.parent() {
            fs::create_dir_all(parent)?;
        }

        fs::write(&self.hook_path, contents).map_err(AdapterError::from)
    }

    fn marker_bounds(contents: &str) -> Option<(usize, usize)> {
        let begin = contents.find(BEGIN_MARKER)?;
        let end_start = contents[begin..].find(END_MARKER)? + begin;
        let after_end = end_start + END_MARKER.len();
        let end = if contents[after_end..].starts_with('\n') {
            after_end + 1
        } else {
            after_end
        };

        Some((begin, end))
    }

    fn append_managed_region(&self, contents: &str) -> String {
        let managed_region = self.managed_region();
        let mut rendered = String::with_capacity(contents.len() + managed_region.len() + 1);
        rendered.push_str(contents);
        if !contents.is_empty() && !contents.ends_with('\n') {
            rendered.push('\n');
        }
        rendered.push_str(&managed_region);
        rendered
    }

    fn render_installed_contents(&self, existing: Option<&str>) -> Result<String, AdapterError> {
        let managed_region = self.managed_region();

        match existing {
            None => Ok(managed_region),
            Some(contents) => match Self::marker_bounds(contents) {
                Some((start, end)) => {
                    let current_region = &contents[start..end];
                    if current_region == managed_region {
                        Ok(contents.to_string())
                    } else {
                        Err(AdapterError::conflict(
                            "managed Kimi hook was modified outside execmanager; manual resolution required",
                        ))
                    }
                }
                None => {
                    let mut rendered =
                        String::with_capacity(contents.len() + managed_region.len() + 1);
                    rendered.push_str(contents);
                    if !contents.is_empty() && !contents.ends_with('\n') {
                        rendered.push('\n');
                    }
                    rendered.push_str(&managed_region);
                    Ok(rendered)
                }
            },
        }
    }

    pub fn install_managed_hook_with_outcome(&self) -> Result<HookInstallOutcome, AdapterError> {
        let snapshot = self.snapshot_existing_hook()?;

        let (rendered, previous_contents, install_mode) = match snapshot {
            HookSnapshot::NoFile => (self.managed_region(), None, HookInstallMode::NewFile),
            HookSnapshot::ExistingNonManaged { contents } => (
                self.append_managed_region(&contents),
                Some(contents),
                HookInstallMode::AppendManagedRegion,
            ),
            HookSnapshot::MatchingManaged { contents } => (
                contents.clone(),
                Some(contents),
                HookInstallMode::AppendManagedRegion,
            ),
            HookSnapshot::ConflictingManaged => {
                return Err(AdapterError::conflict(
                    "managed Kimi hook was modified outside execmanager; manual resolution required",
                ));
            }
        };

        self.write_hook(&rendered)?;

        Ok(HookInstallOutcome {
            hook_path: self.hook_path.clone(),
            previous_contents,
            install_mode,
        })
    }

    fn remove_managed_region(&self, contents: &str) -> Result<String, AdapterError> {
        match Self::marker_bounds(contents) {
            Some((start, end)) => {
                let current_region = &contents[start..end];
                if current_region != self.managed_region() {
                    return Err(AdapterError::conflict(
                        "managed Kimi hook was modified outside execmanager; manual resolution required",
                    ));
                }

                let mut updated = String::with_capacity(contents.len().saturating_sub(end - start));
                updated.push_str(&contents[..start]);
                updated.push_str(&contents[end..]);
                Ok(updated)
            }
            None => Ok(contents.to_string()),
        }
    }
}

impl Adapter for KimiAdapter {
    fn key(&self) -> &'static str {
        "kimi"
    }

    fn detect(&self) -> AdapterDetection {
        match self.hook_path.parent() {
            Some(parent) if parent.exists() => AdapterDetection::Detected {
                hook_path: self.hook_path.clone(),
            },
            Some(parent) => AdapterDetection::NotDetected {
                reason: format!("Kimi hooks directory is missing at {}", parent.display()),
            },
            None => AdapterDetection::NotDetected {
                reason: "Kimi hook path has no parent directory".to_string(),
            },
        }
    }

    fn plan_hook_install(&self) -> HookPlan {
        let managed_region_present = self
            .read_existing_hook()
            .ok()
            .flatten()
            .and_then(|contents| Self::marker_bounds(&contents))
            .is_some();

        HookPlan {
            hook_path: self.hook_path.clone(),
            managed_region_present,
        }
    }

    fn install_managed_hook(&self) -> Result<(), AdapterError> {
        self.install_managed_hook_with_outcome().map(|_| ())
    }

    fn repair_managed_hook(&self) -> Result<(), AdapterError> {
        let existing = self.read_existing_hook()?;
        let rendered = self.render_installed_contents(existing.as_deref())?;
        self.write_hook(&rendered)
    }

    fn uninstall_managed_hook(&self) -> Result<(), AdapterError> {
        let Some(existing) = self.read_existing_hook()? else {
            return Ok(());
        };

        let updated = self.remove_managed_region(&existing)?;

        if updated.is_empty() {
            match fs::remove_file(&self.hook_path) {
                Ok(()) => Ok(()),
                Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
                Err(error) => Err(error.into()),
            }
        } else {
            self.write_hook(&updated)
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::test_support::env::{lock as env_lock, EnvVarGuard};

    use super::KimiAdapter;

    #[test]
    fn new_fails_closed_when_home_is_missing() {
        let _lock = env_lock();
        let _home = EnvVarGuard::remove("HOME");

        let adapter = KimiAdapter::new();
        assert!(
            adapter.is_err(),
            "adapter construction should fail without HOME"
        );
    }
}

#[cfg(test)]
pub mod env {
    use std::{
        ffi::{OsStr, OsString},
        sync::{Mutex, MutexGuard, OnceLock},
    };

    pub struct EnvVarGuard {
        name: &'static str,
        original: Option<OsString>,
    }

    impl EnvVarGuard {
        pub fn set(name: &'static str, value: impl AsRef<OsStr>) -> Self {
            let original = std::env::var_os(name);

            unsafe {
                std::env::set_var(name, value);
            }

            Self { name, original }
        }

        pub fn remove(name: &'static str) -> Self {
            let original = std::env::var_os(name);

            unsafe {
                std::env::remove_var(name);
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

    pub fn lock() -> MutexGuard<'static, ()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();

        LOCK.get_or_init(|| Mutex::new(()))
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

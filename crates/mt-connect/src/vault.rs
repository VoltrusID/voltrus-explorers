//! Secret storage. Tokens never touch a plain file: on macOS and iOS they live in the Keychain.

use std::collections::HashMap;

use parking_lot::Mutex;

use crate::error::ConnectError;

pub trait Vault: Send + Sync {
    fn get(&self, key: &str) -> Result<Option<String>, ConnectError>;
    fn set(&self, key: &str, value: &str) -> Result<(), ConnectError>;
    fn delete(&self, key: &str) -> Result<(), ConnectError>;
}

/// Process memory only. Used for tests and on platforms without a Keychain (the apps ship on macOS and
/// iOS; elsewhere a sign-in lasts until the app quits).
#[derive(Default)]
pub struct MemoryVault(Mutex<HashMap<String, String>>);

impl Vault for MemoryVault {
    fn get(&self, key: &str) -> Result<Option<String>, ConnectError> {
        Ok(self.0.lock().get(key).cloned())
    }
    fn set(&self, key: &str, value: &str) -> Result<(), ConnectError> {
        self.0.lock().insert(key.to_string(), value.to_string());
        Ok(())
    }
    fn delete(&self, key: &str) -> Result<(), ConnectError> {
        self.0.lock().remove(key);
        Ok(())
    }
}

#[cfg(any(target_os = "macos", target_os = "ios"))]
pub use keychain::KeychainVault;

#[cfg(any(target_os = "macos", target_os = "ios"))]
mod keychain {
    use super::{ConnectError, Vault};
    use security_framework::passwords::{delete_generic_password, get_generic_password, set_generic_password};

    /// `errSecItemNotFound`
    const NOT_FOUND: i32 = -25300;

    /// Generic-password items in the app's own Keychain (sandbox-safe: no access group needed).
    pub struct KeychainVault {
        service: String,
    }

    impl KeychainVault {
        pub fn new(service: impl Into<String>) -> Self {
            Self { service: service.into() }
        }
    }

    fn fail(e: security_framework::base::Error) -> ConnectError {
        ConnectError::Store(format!("Keychain: {e}"))
    }

    impl Vault for KeychainVault {
        fn get(&self, key: &str) -> Result<Option<String>, ConnectError> {
            match get_generic_password(&self.service, key) {
                Ok(bytes) => Ok(String::from_utf8(bytes).ok()),
                Err(e) if e.code() == NOT_FOUND => Ok(None),
                Err(e) => Err(fail(e)),
            }
        }
        fn set(&self, key: &str, value: &str) -> Result<(), ConnectError> {
            set_generic_password(&self.service, key, value.as_bytes()).map_err(fail)
        }
        fn delete(&self, key: &str) -> Result<(), ConnectError> {
            match delete_generic_password(&self.service, key) {
                Ok(()) => Ok(()),
                Err(e) if e.code() == NOT_FOUND => Ok(()),
                Err(e) => Err(fail(e)),
            }
        }
    }
}

/// The platform's secret store for `service`.
pub fn platform_vault(service: &str) -> Box<dyn Vault> {
    #[cfg(any(target_os = "macos", target_os = "ios"))]
    {
        Box::new(KeychainVault::new(service))
    }
    #[cfg(not(any(target_os = "macos", target_os = "ios")))]
    {
        let _ = service;
        Box::new(MemoryVault::default())
    }
}

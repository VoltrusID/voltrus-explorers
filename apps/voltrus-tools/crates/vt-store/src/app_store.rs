//! StoreKit 2 through the Swift bridge (`swift/StoreKitBridge.swift`), in
//! store builds for macOS. Other builds sell nothing: every call fails.
//!
//! Every call blocks until the App Store answers (a purchase waits for the
//! user), so call them from a background thread, never the UI thread.

use crate::{Outcome, PRODUCTS, Snapshot, StoreError};

/// The verified entitlements and the prices. Works offline: StoreKit answers
/// entitlements from its on-device cache, prices are then unknown.
pub fn snapshot() -> Result<Snapshot, StoreError> {
    Snapshot::from_bridge(&bridge::snapshot(&products_json()))
}

/// Buy `product` (one of [`PRODUCTS`]).
pub fn purchase(product: &str) -> Result<Outcome, StoreError> {
    Outcome::from_bridge(&bridge::purchase(product))
}

/// Restore purchases: sync with the App Store (it may ask the user to sign
/// in), then answer like [`snapshot`].
pub fn restore() -> Result<Snapshot, StoreError> {
    Snapshot::from_bridge(&bridge::restore(&products_json()))
}

/// Call `on_update` (from a StoreKit thread) whenever a transaction arrives
/// outside [`purchase`]: an Ask to Buy approval, a purchase on another Mac, a
/// refund. Call once per process.
pub fn listen(on_update: extern "C" fn()) {
    bridge::listen(on_update);
}

fn products_json() -> String {
    serde_json::to_string(&PRODUCTS).unwrap_or_else(|_| "[]".to_owned())
}

#[cfg(all(feature = "store", target_os = "macos"))]
#[allow(unsafe_code)]
mod bridge {
    use std::ffi::{CStr, CString, c_char};

    unsafe extern "C" {
        fn vt_sk_snapshot(ids_json: *const c_char) -> *mut c_char;
        fn vt_sk_purchase(id: *const c_char) -> *mut c_char;
        fn vt_sk_restore(ids_json: *const c_char) -> *mut c_char;
        fn vt_sk_listen(callback: extern "C" fn());
        fn vt_sk_free(pointer: *mut c_char);
    }

    /// Call one bridge function with a string argument and take its JSON
    /// answer.
    fn call(function: unsafe extern "C" fn(*const c_char) -> *mut c_char, arg: &str) -> String {
        let Ok(arg) = CString::new(arg) else {
            return r#"{"error":"bad argument"}"#.to_owned();
        };
        // SAFETY: `arg` is a valid NUL-terminated string that outlives the
        // call; the bridge returns a strdup'd NUL-terminated string, which is
        // copied and handed back to vt_sk_free exactly once.
        unsafe {
            let answer = function(arg.as_ptr());
            if answer.is_null() {
                return r#"{"error":"no answer from the App Store"}"#.to_owned();
            }
            let text = CStr::from_ptr(answer).to_string_lossy().into_owned();
            vt_sk_free(answer);
            text
        }
    }

    pub fn snapshot(ids_json: &str) -> String {
        call(vt_sk_snapshot, ids_json)
    }

    pub fn purchase(product: &str) -> String {
        call(vt_sk_purchase, product)
    }

    pub fn restore(ids_json: &str) -> String {
        call(vt_sk_restore, ids_json)
    }

    pub fn listen(callback: extern "C" fn()) {
        // SAFETY: `callback` is a plain function pointer, valid for the life
        // of the process.
        unsafe { vt_sk_listen(callback) }
    }
}

/// Source builds, and other systems: nothing is sold.
#[cfg(not(all(feature = "store", target_os = "macos")))]
mod bridge {
    const UNAVAILABLE: &str = "This build of Voltrus Tools sells nothing.";

    pub fn snapshot(_: &str) -> String {
        format!(r#"{{"error":"{UNAVAILABLE}"}}"#)
    }

    pub fn purchase(_: &str) -> String {
        format!(r#"{{"result":"error","message":"{UNAVAILABLE}"}}"#)
    }

    pub fn restore(_: &str) -> String {
        snapshot("")
    }

    pub fn listen(_: extern "C" fn()) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[cfg(not(feature = "store"))]
    fn source_builds_sell_nothing() {
        assert!(matches!(snapshot(), Err(StoreError::Failed(_))));
        assert!(matches!(restore(), Err(StoreError::Failed(_))));
        assert!(matches!(
            purchase(crate::UNLOCK_PRODUCT),
            Err(StoreError::Failed(_))
        ));
    }

    #[test]
    fn product_ids_go_to_the_bridge_as_json() {
        assert_eq!(
            products_json(),
            r#"["com.joyodigitama.voltrustools.trial14","com.joyodigitama.voltrustools.unlock"]"#
        );
    }
}

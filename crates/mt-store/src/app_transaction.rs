//! StoreKit 2 `AppTransaction`: the verified original purchase date of the app and the App Store environment
//! it runs in. The receipt cannot supply the date (see `swift/AppTransaction.swift`).

/// What StoreKit says about the first acquisition of this app by the signed-in Apple ID.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AppTransactionInfo {
    /// Unix seconds of the original purchase (or free download).
    pub original_purchase: i64,
    /// App Review, TestFlight or an Xcode run: anything but the production App Store.
    pub sandbox: bool,
}

#[cfg(all(feature = "store", any(target_os = "macos", target_os = "ios")))]
mod native {
    use super::AppTransactionInfo;
    use std::sync::OnceLock;

    unsafe extern "C" {
        fn mt_store_app_transaction(out_date: *mut f64, out_sandbox: *mut i32) -> i32;
    }

    static CACHE: OnceLock<AppTransactionInfo> = OnceLock::new();

    /// Blocks while StoreKit answers; call from a blocking thread. Only a successful answer is cached, so an
    /// offline first launch retries on the next status query.
    pub fn fetch() -> Option<AppTransactionInfo> {
        if let Some(info) = CACHE.get() {
            return Some(*info);
        }
        let (mut date, mut sandbox) = (0f64, 0i32);
        // SAFETY: the Swift side writes both out-parameters only when it returns 1.
        let ok = unsafe { mt_store_app_transaction(&mut date, &mut sandbox) };
        if ok != 1 || !date.is_finite() {
            return None;
        }
        let info = AppTransactionInfo { original_purchase: date.floor() as i64, sandbox: sandbox != 0 };
        Some(*CACHE.get_or_init(|| info))
    }
}

#[cfg(all(feature = "store", any(target_os = "macos", target_os = "ios")))]
pub use native::fetch;

#[cfg(not(all(feature = "store", any(target_os = "macos", target_os = "ios"))))]
pub fn fetch() -> Option<AppTransactionInfo> {
    None
}

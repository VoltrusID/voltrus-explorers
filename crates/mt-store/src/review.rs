//! Native App Store rating request (`SKStoreReviewController`).
//!
//! Apple decides whether the sheet shows and caps it at three a year, so this only asks. Real only in App Store
//! builds (`store` feature) on macOS and iOS; everywhere else it does nothing and reports `false`.

/// Asks StoreKit for a rating prompt. Returns whether the request was handed to StoreKit: `false` for builds
/// that are not App Store builds, and on iOS when no foreground-active window scene exists.
#[tauri::command]
pub async fn request_review(app: tauri::AppHandle) -> Result<bool, String> {
    imp::request(app).await
}

#[cfg(all(feature = "store", any(target_os = "macos", target_os = "ios")))]
mod imp {
    pub async fn request(app: tauri::AppHandle) -> Result<bool, String> {
        let (tx, rx) = std::sync::mpsc::channel();
        app.run_on_main_thread(move || {
            let _ = tx.send(request_on_main());
        })
        .map_err(|e| e.to_string())?;
        // The main thread answers at once and never waits on this runtime thread.
        rx.recv().map_err(|e| e.to_string())
    }

    #[cfg(target_os = "macos")]
    #[allow(deprecated)]
    fn request_on_main() -> bool {
        // SAFETY: class method with no arguments, called on the main thread.
        unsafe { objc2_store_kit::SKStoreReviewController::requestReview() };
        true
    }

    #[cfg(target_os = "ios")]
    #[allow(deprecated)]
    fn request_on_main() -> bool {
        use objc2::{ClassType, MainThreadMarker, msg_send};
        use objc2_store_kit::SKStoreReviewController;
        use objc2_ui_kit::{UIApplication, UISceneActivationState, UIWindowScene};

        let Some(mtm) = MainThreadMarker::new() else { return false };
        let scene = UIApplication::sharedApplication(mtm)
            .connectedScenes()
            .iter()
            .filter_map(|s| s.downcast::<UIWindowScene>().ok())
            .find(|s| s.activationState() == UISceneActivationState::ForegroundActive);
        let Some(scene) = scene else { return false };
        // SAFETY: `+[SKStoreReviewController requestReviewInScene:]` takes a UIWindowScene and returns void.
        unsafe {
            let _: () = msg_send![SKStoreReviewController::class(), requestReviewInScene: &*scene];
        }
        true
    }
}

#[cfg(not(all(feature = "store", any(target_os = "macos", target_os = "ios"))))]
mod imp {
    pub async fn request(_app: tauri::AppHandle) -> Result<bool, String> {
        Ok(false)
    }
}

//! Pro entitlement for the explorer apps.
//!
//! - Source and GitHub builds (no feature): everything is unlocked.
//! - App Store builds (`store` feature): Pro comes from the one-time in-app purchase, or from having first
//!   downloaded the app while it was paid ("grandfathered"). The cut-over is configured per app in
//!   `src-tauri/store.json`; while `free_since` is unset the app is still paid, so every store install is Pro.
//! - Direct-sale builds (`keys` feature, the Windows installer): Pro unlocks with a Voltrus licence key,
//!   activated online once (see [`key`]). Configured by the app's `keys.json`; store builds never contain it.
//!
//! Apps register [`setup`] on their builder and the commands; the frontend only calls `store_status`,
//! `store_purchase`, `store_restore` and, in `keys` builds, `store_activate` and `store_deactivate`.

#[cfg(all(feature = "store", feature = "keys"))]
compile_error!("the `store` and `keys` features are different ways to sell Pro; enable one");

#[cfg(feature = "keys")]
pub mod key;
mod receipt;

use serde::{Deserialize, Serialize};

pub use receipt::{installed_original_version, original_application_version};

#[derive(Debug, Clone, Deserialize)]
pub struct StoreConfig {
    /// Non-consumable product id in App Store Connect.
    pub pro_product_id: String,
    #[serde(default)]
    pub free_since: FreeSince,
}

/// First release that was free to download. Buyers whose original download predates it keep Pro.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct FreeSince {
    /// macOS receipts carry the `CFBundleShortVersionString` the user first downloaded, e.g. "1.4.0".
    pub macos_version: Option<String>,
    /// iOS receipts carry the `CFBundleVersion` (our build numbers are Unix times); any value between the
    /// last paid build and the first free build works, e.g. the time the switch is committed.
    pub ios_build: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ProReason {
    /// Source or GitHub build: no store, nothing locked.
    Build,
    /// First downloaded while the app was paid.
    Grandfathered,
    /// Owns the Pro in-app purchase.
    Purchased,
    /// Activated with a licence key.
    Key,
    /// Free tier.
    None,
}

#[derive(Debug, Clone, Serialize)]
pub struct StoreStatus {
    /// Pro is sold in this build (App Store build).
    pub store: bool,
    pub pro: bool,
    pub reason: ProReason,
    /// Localized Pro price, when the store answered.
    pub price: Option<String>,
    /// Pro is unlocked by entering a licence key (`store_activate`) instead of a store purchase.
    pub activation: bool,
    /// Where to buy a key, for builds with `activation`.
    pub buy_url: Option<String>,
}

/// Dotted numeric comparison ("1.10" > "1.9"); missing or non-numeric components count as 0.
pub fn version_lt(a: &str, b: &str) -> bool {
    let parse = |s: &str| s.trim().split('.').map(|p| p.parse::<u64>().unwrap_or(0)).collect::<Vec<_>>();
    let (a, b) = (parse(a), parse(b));
    for i in 0..a.len().max(b.len()) {
        let (x, y) = (a.get(i).copied().unwrap_or(0), b.get(i).copied().unwrap_or(0));
        if x != y {
            return x < y;
        }
    }
    false
}

/// Whether a store install keeps Pro because it was first downloaded while the app was paid.
/// `original` is the receipt's original application version; `None` means no readable receipt.
pub fn grandfathered(free_since: &FreeSince, original: Option<&str>, ios: bool) -> bool {
    if ios {
        match free_since.ios_build {
            None => true,
            Some(cut) => original.and_then(|v| v.trim().parse::<u64>().ok()).is_some_and(|b| b < cut),
        }
    } else {
        match &free_since.macos_version {
            None => true,
            Some(cut) => original.is_some_and(|v| version_lt(v, cut)),
        }
    }
}

/// Registers the store configuration (and the StoreKit plugin in store builds).
pub fn setup<R: tauri::Runtime>(builder: tauri::Builder<R>, config_json: &str) -> tauri::Builder<R> {
    let config: StoreConfig = serde_json::from_str(config_json).expect("invalid store.json");
    let builder = builder.manage(config);
    #[cfg(feature = "store")]
    let builder = builder.plugin(tauri_plugin_iap::init());
    builder
}

/// Registers the licence-key configuration (`keys.json`) of a direct-sale build.
#[cfg(feature = "keys")]
pub fn setup_keys<R: tauri::Runtime>(builder: tauri::Builder<R>, config_json: &str) -> tauri::Builder<R> {
    builder.manage(serde_json::from_str::<key::KeyConfig>(config_json).expect("invalid keys.json"))
}

#[cfg(not(any(feature = "store", feature = "keys")))]
mod commands {
    use super::{ProReason, StoreStatus};

    const NOT_SOLD: &str = "Everything is already unlocked in this build.";

    #[tauri::command]
    pub async fn store_status() -> Result<StoreStatus, String> {
        Ok(StoreStatus { store: false, pro: true, reason: ProReason::Build, price: None, activation: false, buy_url: None })
    }

    #[tauri::command]
    pub async fn store_purchase() -> Result<StoreStatus, String> {
        Err(NOT_SOLD.into())
    }

    #[tauri::command]
    pub async fn store_restore() -> Result<StoreStatus, String> {
        Err(NOT_SOLD.into())
    }

    #[tauri::command]
    pub async fn store_activate(_key: String) -> Result<StoreStatus, String> {
        Err(NOT_SOLD.into())
    }

    #[tauri::command]
    pub async fn store_deactivate() -> Result<StoreStatus, String> {
        Err(NOT_SOLD.into())
    }
}

#[cfg(feature = "keys")]
mod commands {
    use super::{ProReason, StoreStatus, key};
    use key::KeyConfig;
    use std::sync::atomic::{AtomicBool, Ordering};
    use tauri::{AppHandle, State};

    fn status(app: &AppHandle, cfg: &KeyConfig) -> Result<StoreStatus, String> {
        let activation = key::read_activation(&key::license_dir(app)?);
        // No readable machine id (never on Windows) means no unlock: the stored file cannot be tied to this PC.
        let device = key::machine_id().map(|m| key::device_id(&cfg.app, &m)).unwrap_or_default();
        let pro = key::unlocks(activation.as_ref(), &device);
        Ok(StoreStatus {
            store: true,
            pro,
            reason: if pro { ProReason::Key } else { ProReason::None },
            price: cfg.price.clone(),
            activation: true,
            buy_url: Some(cfg.buy_url.clone()),
        })
    }

    /// First status of a launch: ask the server whether this PC still holds the licence. A definitive "no" (the
    /// licence was moved to another device, refunded or expired) removes the stored activation, so Pro locks
    /// again. Offline or any other non-answer changes nothing, which keeps the app usable without internet.
    static CHECKED: AtomicBool = AtomicBool::new(false);

    #[tauri::command]
    pub async fn store_status(app: AppHandle, cfg: State<'_, KeyConfig>) -> Result<StoreStatus, String> {
        let dir = key::license_dir(&app)?;
        if !CHECKED.swap(true, Ordering::Relaxed) {
            if let Some(activation) = key::read_activation(&dir) {
                if let key::Standing::Gone(_) = key::validate(&cfg, &activation, &app.package_info().version.to_string()).await {
                    key::remove_activation(&dir)?;
                }
            }
        }
        status(&app, &cfg)
    }

    /// Pro is bought on the web, not in the app: the frontend opens `buy_url` and asks for the key.
    #[tauri::command]
    pub async fn store_purchase() -> Result<StoreStatus, String> {
        Err("Buy a licence key on voltrus.id, then enter it here.".into())
    }

    #[tauri::command]
    pub async fn store_restore() -> Result<StoreStatus, String> {
        Err("Enter your licence key to unlock Pro on this PC.".into())
    }

    #[tauri::command]
    pub async fn store_activate(
        app: AppHandle,
        cfg: State<'_, KeyConfig>,
        key: String,
        relocate: Option<bool>,
    ) -> Result<StoreStatus, String> {
        let dir = key::license_dir(&app)?;
        key::activate(&cfg, &dir, &key, &app.package_info().version.to_string(), relocate.unwrap_or(false)).await?;
        status(&app, &cfg)
    }

    /// Removes the key from this PC (Pro locks again). The server keeps the device until it is released on the
    /// Licenses page of the console or the licence is moved to another device.
    #[tauri::command]
    pub async fn store_deactivate(app: AppHandle, cfg: State<'_, KeyConfig>) -> Result<StoreStatus, String> {
        key::remove_activation(&key::license_dir(&app)?)?;
        status(&app, &cfg)
    }
}

#[cfg(feature = "store")]
mod commands {
    use super::{ProReason, StoreConfig, StoreStatus, grandfathered, installed_original_version};
    use tauri::{AppHandle, State};
    use tauri_plugin_iap::{IapExt, PurchaseRequest, PurchaseStateValue};

    const INAPP: &str = "inapp";

    async fn status(app: &AppHandle, config: &StoreConfig) -> Result<StoreStatus, String> {
        let original = installed_original_version();
        if grandfathered(&config.free_since, original.as_deref(), cfg!(target_os = "ios")) {
            return Ok(StoreStatus { store: true, pro: true, reason: ProReason::Grandfathered, price: None, activation: false, buy_url: None });
        }
        // StoreKit 2 answers from its local entitlement cache, so this also works offline.
        let owned = app
            .iap()
            .get_product_status(config.pro_product_id.clone(), INAPP.into())
            .await
            .map_err(|e| e.to_string())?;
        if owned.is_owned && owned.purchase_state == Some(PurchaseStateValue::Purchased) {
            return Ok(StoreStatus { store: true, pro: true, reason: ProReason::Purchased, price: None, activation: false, buy_url: None });
        }
        let price = app
            .iap()
            .get_products(vec![config.pro_product_id.clone()], INAPP.into())
            .await
            .ok()
            .and_then(|r| r.products.into_iter().next())
            .and_then(|p| p.formatted_price);
        Ok(StoreStatus { store: true, pro: false, reason: ProReason::None, price, activation: false, buy_url: None })
    }

    #[tauri::command]
    pub async fn store_status(app: AppHandle, config: State<'_, StoreConfig>) -> Result<StoreStatus, String> {
        status(&app, &config).await
    }

    #[tauri::command]
    pub async fn store_purchase(app: AppHandle, config: State<'_, StoreConfig>) -> Result<StoreStatus, String> {
        app.iap()
            .purchase(PurchaseRequest {
                product_id: config.pro_product_id.clone(),
                product_type: INAPP.into(),
                options: None,
            })
            .await
            .map_err(|e| e.to_string())?;
        // Pending (Ask to Buy) and cancelled purchases simply leave Pro locked.
        status(&app, &config).await
    }

    #[tauri::command]
    pub async fn store_restore(app: AppHandle, config: State<'_, StoreConfig>) -> Result<StoreStatus, String> {
        app.iap().restore_purchases(INAPP.into()).await.map_err(|e| e.to_string())?;
        status(&app, &config).await
    }

    #[tauri::command]
    pub async fn store_activate(_key: String) -> Result<StoreStatus, String> {
        Err("Licence keys are not used in the App Store build.".into())
    }

    #[tauri::command]
    pub async fn store_deactivate() -> Result<StoreStatus, String> {
        Err("Licence keys are not used in the App Store build.".into())
    }
}

pub use commands::{store_activate, store_deactivate, store_purchase, store_restore, store_status};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_order_is_numeric() {
        assert!(version_lt("1.3.4", "1.4.0"));
        assert!(version_lt("1.9", "1.10"));
        assert!(version_lt("1.3", "1.3.1"));
        assert!(!version_lt("1.4.0", "1.4"));
        assert!(!version_lt("2.0", "1.99.99"));
    }

    #[test]
    fn paid_app_keeps_every_store_install_pro() {
        let paid = FreeSince::default();
        assert!(grandfathered(&paid, None, false));
        assert!(grandfathered(&paid, None, true));
    }

    #[test]
    fn macos_cutover_uses_the_marketing_version() {
        let fs = FreeSince { macos_version: Some("1.4.0".into()), ios_build: None };
        assert!(grandfathered(&fs, Some("1.3.4"), false));
        assert!(grandfathered(&fs, Some("1.0"), false));
        assert!(!grandfathered(&fs, Some("1.4.0"), false));
        assert!(!grandfathered(&fs, Some("1.5"), false));
        assert!(!grandfathered(&fs, None, false), "no receipt proves nothing");
    }

    #[test]
    fn ios_cutover_uses_the_build_number() {
        let fs = FreeSince { macos_version: Some("1.4.0".into()), ios_build: Some(1_800_000_000) };
        assert!(grandfathered(&fs, Some("1792556911"), true));
        assert!(!grandfathered(&fs, Some("1800000001"), true));
        assert!(!grandfathered(&fs, Some("1.3.5"), true), "a version string is not a build number");
        assert!(!grandfathered(&fs, None, true));
    }

    #[test]
    fn store_json_parses_with_and_without_cutover() {
        let paid: StoreConfig = serde_json::from_str(r#"{"pro_product_id":"x.pro"}"#).unwrap();
        assert!(paid.free_since.macos_version.is_none());
        let free: StoreConfig = serde_json::from_str(
            r#"{"pro_product_id":"x.pro","free_since":{"macos_version":"1.4.0","ios_build":1800000000}}"#,
        )
        .unwrap();
        assert_eq!(free.free_since.ios_build, Some(1_800_000_000));
    }
}

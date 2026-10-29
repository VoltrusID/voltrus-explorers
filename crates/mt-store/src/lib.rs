//! Pro entitlement for the explorer apps.
//!
//! - Source and GitHub builds (no feature): everything is unlocked.
//! - App Store builds (`store` feature): Pro comes from the one-time in-app purchase, or from having first
//!   downloaded the app while it was paid ("grandfathered"), or from having bought it after it went paid
//!   again (`paid_since`). The cut-overs are configured per app in `src-tauri/store.json`; while `free_since`
//!   is unset the app is still paid, so every store install is Pro.
//! - Direct-sale builds (`keys` feature, the Windows installer): Pro unlocks with a Voltrus licence key,
//!   activated online once (see [`key`]). Configured by the app's `keys.json`; store builds never contain it.
//! - Direct-download builds (`license` feature, the macOS DMGs of Serial Monitor and BLE Inspector): Pro unlocks
//!   with a licence key bought in the Voltrus console and activated against the app's download worker, the key
//!   kept in the Keychain (see [`license`]). The `license` section of `store.json` names the worker and the
//!   checkout page.
//!
//! Apps register [`setup`] on their builder and the commands; the frontend only calls `store_status`,
//! `store_purchase`, `store_restore`, `request_review` (App Store builds only do anything) and, in `keys` and `license` builds, `store_activate` and `store_deactivate`.

#[cfg(any(
    all(feature = "store", feature = "keys"),
    all(feature = "store", feature = "license"),
    all(feature = "keys", feature = "license")
))]
compile_error!("the `store`, `keys` and `license` features are different ways to sell Pro; enable one");

#[cfg(feature = "keys")]
pub mod key;
#[cfg(feature = "license")]
pub mod license;
#[cfg_attr(not(feature = "store"), allow(dead_code))]
mod app_transaction;
mod receipt;
mod review;

use serde::{Deserialize, Serialize};

pub use receipt::{InstalledReceipt, installed_receipt, original_application_version};
pub use review::request_review;

/// `license` section of `store.json`: how a direct-download build sells Pro by licence key.
#[derive(Debug, Clone, Deserialize)]
pub struct LicenseConfig {
    /// Download worker that activates and validates keys, e.g. `https://dl-serial-monitor.voltrus.id`.
    pub server: String,
    /// Console checkout page of the Pro licence; opened in the browser by the Buy button.
    pub buy_url: String,
    /// Display price of Pro, e.g. `$4.99`.
    #[serde(default)]
    pub price: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct StoreConfig {
    /// Non-consumable product id in App Store Connect.
    pub pro_product_id: String,
    #[serde(default)]
    pub free_since: FreeSince,
    /// Set once a formerly free app is paid again: installs whose original purchase is on or after it are Pro.
    #[serde(default)]
    pub paid_since: Option<PaidSince>,
    /// Licence-key sales; read by `license` builds only.
    #[serde(default)]
    pub license: Option<LicenseConfig>,
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

/// The moment the app became a paid download again.
#[derive(Debug, Clone, Deserialize)]
pub struct PaidSince {
    /// RFC 3339 timestamp, e.g. `2026-10-28T00:00:00Z`.
    pub date: String,
}

impl PaidSince {
    /// Cut-off as Unix seconds; `None` when `date` is not valid RFC 3339.
    pub fn cutoff(&self) -> Option<i64> {
        parse_rfc3339(&self.date)
    }
}

/// Unix seconds of an RFC 3339 timestamp (`2026-10-28T00:00:00Z`, optional fraction, `Z` or `±hh:mm`).
pub fn parse_rfc3339(s: &str) -> Option<i64> {
    let b = s.trim().as_bytes();
    let num = |from: usize, len: usize| -> Option<i64> {
        let d = b.get(from..from + len)?;
        d.iter().all(u8::is_ascii_digit).then(|| d.iter().fold(0i64, |a, c| a * 10 + i64::from(c - b'0')))
    };
    let sep = |at: usize, c: &[u8]| b.get(at).is_some_and(|x| c.contains(x));
    if !(sep(4, b"-") && sep(7, b"-") && sep(10, b"Tt") && sep(13, b":") && sep(16, b":")) {
        return None;
    }
    let (year, month, day) = (num(0, 4)?, num(5, 2)?, num(8, 2)?);
    let (hour, min, sec) = (num(11, 2)?, num(14, 2)?, num(17, 2)?);
    let mut at = 19;
    if sep(at, b".") {
        at += 1;
        let digits = b[at..].iter().take_while(|c| c.is_ascii_digit()).count();
        if digits == 0 {
            return None;
        }
        at += digits;
    }
    let offset = if sep(at, b"Zz") && at + 1 == b.len() {
        0
    } else if sep(at, b"+-") && at + 6 == b.len() && sep(at + 3, b":") {
        let secs = num(at + 1, 2)? * 3600 + num(at + 4, 2)? * 60;
        if b[at] == b'+' { secs } else { -secs }
    } else {
        return None;
    };
    let leap_ok = |y: i64| (y % 4 == 0 && y % 100 != 0) || y % 400 == 0;
    let days_in_month = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 => 28 + i64::from(leap_ok(year)),
        _ => return None,
    };
    if day < 1 || day > days_in_month || hour > 23 || min > 59 || sec > 60 {
        return None;
    }
    // Days since 1970-01-01 (Howard Hinnant's civil-from-days, inverted).
    let y = if month <= 2 { year - 1 } else { year };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * (if month > 2 { month - 3 } else { month + 9 }) + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    Some(days * 86_400 + hour * 3600 + min * 60 + sec - offset)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ProReason {
    /// Source or CI build: no store, nothing locked.
    Build,
    /// First downloaded while the app was paid.
    Grandfathered,
    /// Bought after the app became paid again (`paid_since`), or a sandbox install of such an app.
    Paid,
    /// Owns the Pro in-app purchase.
    Purchased,
    /// Activated with a licence key.
    Key,
    /// Free tier.
    None,
}

#[derive(Debug, Clone, Serialize)]
pub struct StoreStatus {
    /// Pro is sold in this build through the App Store, or by licence key in the Windows build (`keys`).
    pub store: bool,
    pub pro: bool,
    pub reason: ProReason,
    /// Localized Pro price, when the store answered (App Store) or the configured display price (licence key).
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

/// Whether the app was acquired on or after the paid switch. Without `paid_since` nobody qualifies; a missing or
/// unreadable purchase date or a malformed cut-off proves nothing. Sandbox installs (App Review, TestFlight, Xcode)
/// carry a synthetic purchase date, so for a paid app they count as bought: reviewers see everything unlocked.
pub fn bought_while_paid(paid_since: Option<&PaidSince>, original_purchase: Option<i64>, sandbox: bool) -> bool {
    let Some(cutoff) = paid_since.and_then(PaidSince::cutoff) else { return false };
    sandbox || original_purchase.is_some_and(|d| d >= cutoff)
}

/// Pro for a store install, in order: grandfathered by `free_since`, bought while paid, Pro IAP owned.
/// A sandbox install ignores `original` (Apple reports "1.0" there, which would grandfather the reviewer).
pub fn entitlement(
    config: &StoreConfig,
    original: Option<&str>,
    ios: bool,
    original_purchase: Option<i64>,
    sandbox: bool,
    iap_owned: bool,
) -> ProReason {
    if grandfathered(&config.free_since, original.filter(|_| !sandbox), ios) {
        ProReason::Grandfathered
    } else if bought_while_paid(config.paid_since.as_ref(), original_purchase, sandbox) {
        ProReason::Paid
    } else if iap_owned {
        ProReason::Purchased
    } else {
        ProReason::None
    }
}

/// Registers the store configuration (the StoreKit plugin in store builds, the licence client in licence builds).
pub fn setup<R: tauri::Runtime>(builder: tauri::Builder<R>, config_json: &str) -> tauri::Builder<R> {
    let config: StoreConfig = serde_json::from_str(config_json).expect("invalid store.json");
    if let Some(paid) = &config.paid_since {
        assert!(paid.cutoff().is_some(), "store.json paid_since.date must be RFC 3339, got {:?}", paid.date);
    }
    #[cfg(feature = "license")]
    let builder = builder.manage(license::Slot::new(
        config.license.clone().expect("store.json needs a `license` section in licence builds"),
    ));
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

#[cfg(not(any(feature = "store", feature = "keys", feature = "license")))]
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
    use super::{ProReason, StoreConfig, StoreStatus, app_transaction, entitlement, installed_receipt};
    use tauri::{AppHandle, State};
    use tauri_plugin_iap::{IapExt, PurchaseRequest, PurchaseStateValue};

    const INAPP: &str = "inapp";

    fn pro(reason: ProReason) -> StoreStatus {
        StoreStatus { store: true, pro: true, reason, price: None, activation: false, buy_url: None }
    }

    async fn status(app: &AppHandle, config: &StoreConfig) -> Result<StoreStatus, String> {
        let receipt = installed_receipt();
        // The original purchase date is only needed (and only asked of StoreKit) for a paid-since app.
        let transaction = if config.paid_since.is_some() {
            tauri::async_runtime::spawn_blocking(app_transaction::fetch).await.ok().flatten()
        } else {
            None
        };
        let sandbox = receipt.sandbox || transaction.is_some_and(|t| t.sandbox);
        let reason = entitlement(
            config,
            receipt.original_version.as_deref(),
            cfg!(target_os = "ios"),
            transaction.map(|t| t.original_purchase),
            sandbox,
            false,
        );
        if reason != ProReason::None {
            return Ok(pro(reason));
        }
        // StoreKit 2 answers from its local entitlement cache, so this also works offline.
        let owned = app
            .iap()
            .get_product_status(config.pro_product_id.clone(), INAPP.into())
            .await
            .map_err(|e| e.to_string())?;
        if owned.is_owned && owned.purchase_state == Some(PurchaseStateValue::Purchased) {
            return Ok(pro(ProReason::Purchased));
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

#[cfg(feature = "license")]
mod commands {
    use super::license::{Entitlement, Slot};
    use super::{ProReason, StoreStatus};
    use tauri::{AppHandle, State};

    /// A licence-key DMG is not an App Store build (`store: false`: no sandbox, so the virtual port and the source
    /// link stay), but the frontend gets the same key flow as the Windows build: `activation` and `buy_url`.
    fn status_of(slot: &Slot, e: Entitlement) -> StoreStatus {
        StoreStatus {
            store: false,
            pro: e.pro,
            reason: if e.pro { ProReason::Key } else { ProReason::None },
            price: slot.config.price.clone(),
            activation: true,
            buy_url: Some(slot.config.buy_url.clone()),
        }
    }

    #[tauri::command]
    pub async fn store_status(app: AppHandle, slot: State<'_, Slot>) -> Result<StoreStatus, String> {
        let e = slot.licence(&app).await?.status().await;
        Ok(status_of(&slot, e))
    }

    /// Pro is bought on the web, not in the app: the frontend opens `buy_url` and asks for the key.
    #[tauri::command]
    pub async fn store_purchase() -> Result<StoreStatus, String> {
        Err("Buy a licence key in the Voltrus console, then enter it here.".into())
    }

    #[tauri::command]
    pub async fn store_restore() -> Result<StoreStatus, String> {
        Err("Enter your licence key to unlock Pro.".into())
    }

    /// Activates a licence key on this device; the error text is meant for the user. With `relocate` the licence
    /// is moved here: the server releases the device that holds it first.
    #[tauri::command]
    pub async fn store_activate(
        app: AppHandle,
        slot: State<'_, Slot>,
        key: String,
        relocate: Option<bool>,
    ) -> Result<StoreStatus, String> {
        let e = slot.licence(&app).await?.activate(&key, relocate.unwrap_or(false)).await?;
        Ok(status_of(&slot, e))
    }

    /// Forgets the licence key on this device; Pro locks again.
    #[tauri::command]
    pub async fn store_deactivate(app: AppHandle, slot: State<'_, Slot>) -> Result<StoreStatus, String> {
        let e = slot.licence(&app).await?.remove().await?;
        Ok(status_of(&slot, e))
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

    #[test]
    fn store_json_carries_the_licence_section() {
        let c: StoreConfig = serde_json::from_str(
            r#"{"pro_product_id":"x.pro","license":{"server":"https://dl.example","buy_url":"https://console.example/checkout","price":"$4.99"}}"#,
        )
        .unwrap();
        let l = c.license.unwrap();
        assert_eq!((l.server.as_str(), l.price.as_deref()), ("https://dl.example", Some("$4.99")));
        assert_eq!(serde_json::to_value(ProReason::Key).unwrap(), "key");
    }

    const CUTOVER: i64 = 1_793_145_600; // 2026-10-28T00:00:00Z

    fn bacnet_like() -> StoreConfig {
        serde_json::from_str(
            r#"{"pro_product_id":"x.pro","free_since":{"macos_version":"1.2.1","ios_build":1792723289},"paid_since":{"date":"2026-10-28T00:00:00Z"}}"#,
        )
        .unwrap()
    }

    fn rule(c: &StoreConfig, original: Option<&str>, purchase: Option<i64>, sandbox: bool, iap: bool) -> ProReason {
        entitlement(c, original, false, purchase, sandbox, iap)
    }

    #[test]
    fn rfc3339_parses_to_unix_seconds() {
        assert_eq!(parse_rfc3339("2026-10-28T00:00:00Z"), Some(CUTOVER));
        assert_eq!(parse_rfc3339("2026-10-28T07:00:00+07:00"), Some(CUTOVER));
        assert_eq!(parse_rfc3339("2026-10-27T17:00:00-07:00"), Some(CUTOVER));
        assert_eq!(parse_rfc3339("2000-02-29T12:30:45.123Z"), Some(951_827_445));
        assert_eq!(parse_rfc3339("1970-01-01T00:00:00Z"), Some(0));
    }

    #[test]
    fn rfc3339_rejects_malformed_input() {
        for bad in ["", "2026-10-28", "2026-10-28T00:00:00", "2026-13-01T00:00:00Z", "2026-02-30T00:00:00Z", "2026-10-28T24:00:00Z", "yesterday", "2026-10-28T00:00:00.Z", "2026-10-28T00:00:00+0700"] {
            assert_eq!(parse_rfc3339(bad), None, "{bad}");
        }
    }

    #[test]
    fn paid_since_parses_from_store_json_and_is_optional() {
        assert_eq!(bacnet_like().paid_since.unwrap().cutoff(), Some(CUTOVER));
        let none: StoreConfig = serde_json::from_str(r#"{"pro_product_id":"x.pro"}"#).unwrap();
        assert!(none.paid_since.is_none());
    }

    #[test]
    fn bought_before_the_free_window_is_grandfathered() {
        let c = bacnet_like();
        assert_eq!(rule(&c, Some("1.2.0"), Some(CUTOVER - 400 * 86_400), false, false), ProReason::Grandfathered);
    }

    #[test]
    fn free_window_download_needs_the_iap() {
        let c = bacnet_like();
        let during = Some(CUTOVER - 86_400);
        assert_eq!(rule(&c, Some("1.2.3"), during, false, false), ProReason::None);
        assert_eq!(rule(&c, Some("1.2.3"), during, false, true), ProReason::Purchased);
    }

    #[test]
    fn purchase_on_or_after_the_paid_switch_is_pro() {
        let c = bacnet_like();
        assert_eq!(rule(&c, Some("1.2.3"), Some(CUTOVER), false, false), ProReason::Paid, "exactly at the cut-off");
        assert_eq!(rule(&c, Some("1.2.4"), Some(CUTOVER + 30 * 86_400), false, false), ProReason::Paid);
        assert_eq!(rule(&c, Some("1.2.3"), Some(CUTOVER - 1), false, false), ProReason::None, "one second early");
    }

    #[test]
    fn missing_or_malformed_dates_prove_nothing() {
        let c = bacnet_like();
        assert_eq!(rule(&c, Some("1.2.3"), None, false, false), ProReason::None);
        let bad: StoreConfig =
            serde_json::from_str(r#"{"pro_product_id":"x.pro","free_since":{"macos_version":"1.2.1"},"paid_since":{"date":"soon"}}"#).unwrap();
        assert_eq!(bad.paid_since.as_ref().unwrap().cutoff(), None);
        assert_eq!(rule(&bad, Some("1.2.3"), Some(CUTOVER + 1), false, false), ProReason::None);
        assert_eq!(rule(&bad, Some("1.2.3"), Some(CUTOVER + 1), true, false), ProReason::None, "bad cut-off also beats sandbox");
        let unset: StoreConfig = serde_json::from_str(r#"{"pro_product_id":"x.pro","free_since":{"macos_version":"1.2.1"}}"#).unwrap();
        assert_eq!(rule(&unset, Some("1.2.3"), Some(CUTOVER + 1), false, false), ProReason::None, "no paid_since: free-with-IAP app");
    }

    #[test]
    fn sandbox_installs_of_a_paid_app_are_pro_and_never_grandfathered() {
        let c = bacnet_like();
        // App Review / TestFlight: receipt says original version "1.0" (dropped upstream) and a synthetic date.
        assert_eq!(rule(&c, Some("1.0"), None, true, false), ProReason::Paid);
        assert_eq!(rule(&c, Some("1.0"), Some(0), true, false), ProReason::Paid);
        // Before paid_since is configured the sandbox keeps showing the paywall so reviewers can test the IAP.
        let free: StoreConfig = serde_json::from_str(r#"{"pro_product_id":"x.pro","free_since":{"macos_version":"1.2.1"}}"#).unwrap();
        assert_eq!(rule(&free, Some("1.0"), None, true, false), ProReason::None);
    }

    #[test]
    fn paid_app_without_free_since_stays_grandfathered() {
        let c: StoreConfig = serde_json::from_str(r#"{"pro_product_id":"x.pro"}"#).unwrap();
        assert_eq!(rule(&c, None, None, false, false), ProReason::Grandfathered);
        assert_eq!(rule(&c, None, None, true, false), ProReason::Grandfathered, "App Review sees everything");
    }
}

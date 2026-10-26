//! Licensing for Voltrus Tools, without UI.
//!
//! - Source and GitHub builds (no feature): everything is unlocked, nothing is
//!   sold ([`Licence::Unrestricted`]).
//! - Mac App Store builds (`store` feature): a free download with a 14-day
//!   trial, then a one-time unlock. Both are non-consumable in-app purchases,
//!   as App Review guideline 3.1.1 asks of time-based trials in non-subscription
//!   apps: the trial is a free item named "14-day Trial", and its purchase date
//!   starts the trial. After it the app is read-only: saved data and views open,
//!   connecting, simulating and writing don't.
//!
//! [`Licence::evaluate`] turns the verified entitlements and the time into the
//! licence; [`Cache`] keeps the last verified entitlements on disk so the app
//! knows its licence offline and at once on launch, and keeps the latest time
//! the Mac has seen so turning the clock back never extends a trial.
//! [`app_store`] talks to StoreKit 2 in store builds; elsewhere it sells
//! nothing.

use std::fs;
use std::io::{self, Write as _};
use std::path::Path;

use serde::{Deserialize, Serialize};

pub mod app_store;

/// The free "14-day Trial" non-consumable: buying it starts the trial.
pub const TRIAL_PRODUCT: &str = "com.joyodigitama.voltrustools.trial14";
/// The one-time unlock, a non-consumable.
pub const UNLOCK_PRODUCT: &str = "com.joyodigitama.voltrustools.unlock";
pub const PRODUCTS: [&str; 2] = [TRIAL_PRODUCT, UNLOCK_PRODUCT];

pub const TRIAL_DAYS: i64 = 14;
const DAY: i64 = 86_400;
pub const TRIAL_SECONDS: i64 = TRIAL_DAYS * DAY;

/// Whether this binary sells the app through the App Store.
pub const fn store_build() -> bool {
    cfg!(feature = "store")
}

/// What the App Store verified this Apple ID owns. Times are Unix seconds.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entitlements {
    pub unlocked: bool,
    /// Original purchase date of the trial item: the trial's start.
    pub trial_started: Option<i64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Licence {
    /// A source or GitHub build: everything works, nothing is sold.
    Unrestricted,
    /// A store install whose trial hasn't been started.
    NotStarted,
    Trial {
        /// Unix seconds.
        ends: i64,
        /// Whole days left, rounded up: 14 right after starting, 1 on the
        /// last day.
        days_left: i64,
    },
    Expired,
    Unlocked,
}

impl Licence {
    /// The licence at `now` (Unix seconds, already clock-checked by
    /// [`Cache::observe`]).
    pub fn evaluate(entitlements: &Entitlements, now: i64) -> Self {
        if entitlements.unlocked {
            return Self::Unlocked;
        }
        let Some(started) = entitlements.trial_started else {
            return Self::NotStarted;
        };
        let ends = started.saturating_add(TRIAL_SECONDS);
        // A clock behind the App Store's purchase time counts as the moment
        // of purchase: never more than the full trial.
        let now = now.max(started);
        if now < ends {
            Self::Trial {
                ends,
                days_left: (ends - now + DAY - 1) / DAY,
            }
        } else {
            Self::Expired
        }
    }

    /// Whether sessions may connect, simulate and write. Without it the app
    /// is read-only.
    pub fn can_operate(self) -> bool {
        matches!(
            self,
            Self::Unrestricted | Self::Trial { .. } | Self::Unlocked
        )
    }
}

/// The last verified entitlements, saved next to the workspace (inside the
/// app's container in the sandbox).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Cache {
    pub entitlements: Entitlements,
    /// The latest time this Mac has seen, from its clock or from an App Store
    /// signature (Unix seconds). Turning the clock back stops here, so it
    /// neither extends the trial nor locks anyone out.
    pub high_water: i64,
}

impl Cache {
    /// The saved cache; a missing or unreadable file is an empty one (the
    /// next StoreKit answer fills it).
    pub fn load(path: &Path) -> Self {
        fs::read_to_string(path)
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default()
    }

    /// Atomic: write a sibling, then rename.
    pub fn save(&self, path: &Path) -> io::Result<()> {
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir)?;
        }
        let staging = path.with_extension("json.saving");
        let mut file = fs::File::create(&staging)?;
        file.write_all(&serde_json::to_vec_pretty(self).map_err(io::Error::other)?)?;
        file.sync_all()?;
        fs::rename(&staging, path)
    }

    /// Record the clock's `now` and return the time to judge the licence by:
    /// never earlier than any time seen before.
    pub fn observe(&mut self, now: i64) -> i64 {
        self.high_water = self.high_water.max(now);
        self.high_water
    }

    /// The licence at the clock's `now`.
    pub fn licence(&mut self, now: i64) -> Licence {
        let now = self.observe(now);
        Licence::evaluate(&self.entitlements, now)
    }

    /// Take a fresh, verified StoreKit answer.
    ///
    /// The unlock follows StoreKit exactly (a refund locks again). A trial,
    /// once started on this Mac, stays started from its earliest known date:
    /// signing in with another Apple ID doesn't start it over.
    pub fn update(&mut self, fresh: &Snapshot) {
        let trial_started = match (
            self.entitlements.trial_started,
            fresh.entitlements.trial_started,
        ) {
            (Some(cached), Some(fresh)) => Some(cached.min(fresh)),
            (cached, fresh) => cached.or(fresh),
        };
        self.entitlements = Entitlements {
            unlocked: fresh.entitlements.unlocked,
            trial_started,
        };
        if let Some(signed) = fresh.signed {
            self.observe(signed);
        }
    }

    /// A purchase StoreKit just verified.
    pub fn purchased(&mut self, product: &str, original: i64) {
        if product == UNLOCK_PRODUCT {
            self.entitlements.unlocked = true;
        } else if product == TRIAL_PRODUCT {
            self.entitlements.trial_started = Some(
                self.entitlements
                    .trial_started
                    .map_or(original, |started| started.min(original)),
            );
        }
    }
}

/// Prices in the App Store's local currency, formatted ("$49.00"). `None`
/// while unknown (offline).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Prices {
    pub trial: Option<String>,
    pub unlock: Option<String>,
}

/// One StoreKit answer.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Snapshot {
    pub entitlements: Entitlements,
    pub prices: Prices,
    /// The newest App Store signature time among the entitlements.
    pub signed: Option<i64>,
}

#[derive(Deserialize)]
struct BridgeSnapshot {
    #[serde(default)]
    entitlements: Vec<BridgeEntitlement>,
    #[serde(default)]
    prices: std::collections::HashMap<String, String>,
    error: Option<String>,
}

#[derive(Deserialize)]
struct BridgeEntitlement {
    product: String,
    original: i64,
    signed: i64,
}

impl Snapshot {
    /// Parse the bridge's answer (`vt_sk_snapshot` / `vt_sk_restore`).
    pub fn from_bridge(json: &str) -> Result<Self, StoreError> {
        let answer: BridgeSnapshot = serde_json::from_str(json)
            .map_err(|error| StoreError::Failed(format!("unreadable App Store answer: {error}")))?;
        match answer.error.as_deref() {
            Some("cancelled") => return Err(StoreError::Cancelled),
            Some(message) => return Err(StoreError::Failed(message.to_owned())),
            None => {}
        }
        let mut snapshot = Self {
            prices: Prices {
                trial: answer.prices.get(TRIAL_PRODUCT).cloned(),
                unlock: answer.prices.get(UNLOCK_PRODUCT).cloned(),
            },
            ..Self::default()
        };
        for owned in answer.entitlements {
            if owned.product == UNLOCK_PRODUCT {
                snapshot.entitlements.unlocked = true;
            } else if owned.product == TRIAL_PRODUCT {
                snapshot.entitlements.trial_started = Some(
                    snapshot
                        .entitlements
                        .trial_started
                        .map_or(owned.original, |started| started.min(owned.original)),
                );
            } else {
                continue;
            }
            snapshot.signed = Some(
                snapshot
                    .signed
                    .map_or(owned.signed, |s| s.max(owned.signed)),
            );
        }
        Ok(snapshot)
    }
}

/// How a purchase ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// Verified; `original` is the item's first purchase date (Unix seconds).
    Purchased {
        original: i64,
    },
    /// Waiting for approval (Ask to Buy); it arrives later as an update.
    Pending,
    Cancelled,
}

#[derive(Deserialize)]
struct BridgePurchase {
    result: String,
    original: Option<i64>,
    message: Option<String>,
}

impl Outcome {
    /// Parse the bridge's answer to `vt_sk_purchase`.
    pub fn from_bridge(json: &str) -> Result<Self, StoreError> {
        let answer: BridgePurchase = serde_json::from_str(json)
            .map_err(|error| StoreError::Failed(format!("unreadable App Store answer: {error}")))?;
        match (answer.result.as_str(), answer.original) {
            ("purchased", Some(original)) => Ok(Self::Purchased { original }),
            ("pending", _) => Ok(Self::Pending),
            ("cancelled", _) => Ok(Self::Cancelled),
            _ => Err(StoreError::Failed(answer.message.unwrap_or_else(|| {
                "The App Store didn't complete the purchase.".to_owned()
            }))),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StoreError {
    /// The user dismissed the App Store's sign-in or confirmation.
    Cancelled,
    Failed(String),
}

impl std::fmt::Display for StoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Cancelled => f.write_str("Cancelled."),
            Self::Failed(message) => f.write_str(message),
        }
    }
}

impl std::error::Error for StoreError {}

#[cfg(test)]
mod tests {
    use super::*;

    const T0: i64 = 1_790_000_000;

    fn trial(started: i64) -> Entitlements {
        Entitlements {
            unlocked: false,
            trial_started: Some(started),
        }
    }

    #[test]
    fn nothing_bought_is_not_started() {
        assert_eq!(
            Licence::evaluate(&Entitlements::default(), T0),
            Licence::NotStarted
        );
        assert!(!Licence::NotStarted.can_operate());
    }

    #[test]
    fn a_fresh_trial_has_fourteen_days() {
        let licence = Licence::evaluate(&trial(T0), T0);
        assert_eq!(
            licence,
            Licence::Trial {
                ends: T0 + TRIAL_SECONDS,
                days_left: 14
            }
        );
        assert!(licence.can_operate());
    }

    #[test]
    fn days_left_round_up_until_the_last_second() {
        let days = |elapsed: i64| match Licence::evaluate(&trial(T0), T0 + elapsed) {
            Licence::Trial { days_left, .. } => Some(days_left),
            _ => None,
        };
        assert_eq!(days(1), Some(14));
        assert_eq!(days(DAY), Some(13));
        assert_eq!(days(DAY + 1), Some(13));
        assert_eq!(days(13 * DAY), Some(1));
        assert_eq!(days(TRIAL_SECONDS - 1), Some(1));
    }

    #[test]
    fn the_trial_expires_exactly_at_fourteen_days() {
        assert!(Licence::evaluate(&trial(T0), T0 + TRIAL_SECONDS - 1).can_operate());
        assert_eq!(
            Licence::evaluate(&trial(T0), T0 + TRIAL_SECONDS),
            Licence::Expired
        );
        assert!(!Licence::Expired.can_operate());
        assert_eq!(
            Licence::evaluate(&trial(T0), T0 + 400 * DAY),
            Licence::Expired
        );
    }

    #[test]
    fn the_unlock_wins_over_any_trial() {
        let bought = Entitlements {
            unlocked: true,
            trial_started: Some(T0 - 100 * DAY),
        };
        assert_eq!(Licence::evaluate(&bought, T0), Licence::Unlocked);
        assert!(Licence::Unlocked.can_operate());
        assert!(Licence::Unrestricted.can_operate());
    }

    #[test]
    fn a_clock_behind_the_purchase_never_gives_more_than_fourteen_days() {
        let licence = Licence::evaluate(&trial(T0), T0 - 3 * DAY);
        assert_eq!(
            licence,
            Licence::Trial {
                ends: T0 + TRIAL_SECONDS,
                days_left: 14
            }
        );
    }

    #[test]
    fn turning_the_clock_back_does_not_extend_the_trial() {
        let mut cache = Cache {
            entitlements: trial(T0),
            high_water: 0,
        };
        assert_eq!(
            cache.licence(T0 + 10 * DAY),
            Licence::Trial {
                ends: T0 + TRIAL_SECONDS,
                days_left: 4
            }
        );
        // Back to the day of purchase: still 4 days left, not 14.
        assert_eq!(
            cache.licence(T0 + DAY),
            Licence::Trial {
                ends: T0 + TRIAL_SECONDS,
                days_left: 4
            }
        );
        assert_eq!(cache.licence(T0 + 15 * DAY), Licence::Expired);
        // And once expired, going back a week doesn't reopen it.
        assert_eq!(cache.licence(T0 + 8 * DAY), Licence::Expired);
    }

    #[test]
    fn a_clock_turned_back_never_locks_out_a_buyer() {
        let mut cache = Cache {
            entitlements: Entitlements {
                unlocked: true,
                trial_started: None,
            },
            high_water: T0 + 900 * DAY,
        };
        assert_eq!(cache.licence(T0), Licence::Unlocked);
    }

    #[test]
    fn an_app_store_signature_moves_the_clock_floor() {
        let mut cache = Cache::default();
        cache.update(&Snapshot {
            entitlements: trial(T0),
            prices: Prices::default(),
            signed: Some(T0 + 20 * DAY),
        });
        // The local clock says day 2, the App Store signed on day 20.
        assert_eq!(cache.licence(T0 + 2 * DAY), Licence::Expired);
    }

    #[test]
    fn restore_brings_back_the_unlock_and_keeps_the_trial_start() {
        let mut cache = Cache {
            entitlements: trial(T0),
            high_water: T0,
        };
        assert_eq!(cache.licence(T0 + 30 * DAY), Licence::Expired);
        let restored = Snapshot::from_bridge(&format!(
            r#"{{"entitlements":[
                {{"product":"{UNLOCK_PRODUCT}","original":{},"signed":{}}},
                {{"product":"{TRIAL_PRODUCT}","original":{T0},"signed":{}}}
            ],"prices":{{}}}}"#,
            T0 + 20 * DAY,
            T0 + 30 * DAY,
            T0 + 30 * DAY
        ))
        .unwrap();
        cache.update(&restored);
        assert_eq!(cache.licence(T0 + 30 * DAY), Licence::Unlocked);
        assert_eq!(cache.entitlements.trial_started, Some(T0));
    }

    #[test]
    fn a_refund_locks_again() {
        let mut cache = Cache {
            entitlements: Entitlements {
                unlocked: true,
                trial_started: Some(T0),
            },
            high_water: T0,
        };
        let refunded = Snapshot::from_bridge(&format!(
            r#"{{"entitlements":[{{"product":"{TRIAL_PRODUCT}","original":{T0},"signed":{T0}}}]}}"#
        ))
        .unwrap();
        cache.update(&refunded);
        assert_eq!(cache.licence(T0 + 30 * DAY), Licence::Expired);
    }

    #[test]
    fn another_apple_id_does_not_restart_a_trial_on_this_mac() {
        let mut cache = Cache {
            entitlements: trial(T0),
            high_water: T0,
        };
        // Signed in with an Apple ID that never started the trial.
        cache.update(&Snapshot::default());
        assert_eq!(cache.entitlements.trial_started, Some(T0));
        // ... and starting it again there counts from the first start.
        cache.purchased(TRIAL_PRODUCT, T0 + 20 * DAY);
        assert_eq!(cache.licence(T0 + 20 * DAY), Licence::Expired);
    }

    #[test]
    fn purchases_update_the_cache() {
        let mut cache = Cache::default();
        cache.purchased(TRIAL_PRODUCT, T0);
        assert!(cache.licence(T0).can_operate());
        cache.purchased(UNLOCK_PRODUCT, T0 + DAY);
        assert_eq!(cache.licence(T0 + 99 * DAY), Licence::Unlocked);
        // Unknown products change nothing.
        let before = cache;
        cache.purchased("com.example.other", T0);
        assert_eq!(cache, before);
    }

    #[test]
    fn the_cache_survives_a_restart_and_tolerates_a_bad_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join("licence.json");
        assert_eq!(Cache::load(&path), Cache::default());
        let cache = Cache {
            entitlements: trial(T0),
            high_water: T0 + DAY,
        };
        cache.save(&path).unwrap();
        assert_eq!(Cache::load(&path), cache);
        fs::write(&path, "{ not json").unwrap();
        assert_eq!(Cache::load(&path), Cache::default());
    }

    #[test]
    fn bridge_answers_parse() {
        let snapshot = Snapshot::from_bridge(&format!(
            r#"{{"entitlements":[{{"product":"{TRIAL_PRODUCT}","original":{T0},"signed":{}}},
                {{"product":"com.example.other","original":1,"signed":{}}}],
              "prices":{{"{UNLOCK_PRODUCT}":"$49.00","{TRIAL_PRODUCT}":"Free"}}}}"#,
            T0 + 5,
            T0 + 99
        ))
        .unwrap();
        assert_eq!(snapshot.entitlements, trial(T0));
        assert_eq!(snapshot.signed, Some(T0 + 5));
        assert_eq!(snapshot.prices.unlock.as_deref(), Some("$49.00"));
        assert_eq!(snapshot.prices.trial.as_deref(), Some("Free"));
        assert_eq!(
            Snapshot::from_bridge(r#"{"error":"cancelled"}"#),
            Err(StoreError::Cancelled)
        );
        assert_eq!(
            Snapshot::from_bridge(r#"{"error":"offline"}"#),
            Err(StoreError::Failed("offline".into()))
        );

        assert_eq!(
            Outcome::from_bridge(r#"{"result":"purchased","original":5}"#),
            Ok(Outcome::Purchased { original: 5 })
        );
        assert_eq!(
            Outcome::from_bridge(r#"{"result":"pending"}"#),
            Ok(Outcome::Pending)
        );
        assert_eq!(
            Outcome::from_bridge(r#"{"result":"cancelled"}"#),
            Ok(Outcome::Cancelled)
        );
        assert_eq!(
            Outcome::from_bridge(r#"{"result":"error","message":"no"}"#),
            Err(StoreError::Failed("no".into()))
        );
    }
}

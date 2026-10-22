//! Connect subscriptions through StoreKit (App Store builds only).
//!
//! The server owns the entitlement: the app buys the subscription, sends the signed transaction to the
//! signed-in account, and re-reads `/me`. Without the `store` feature these commands exist but refuse.

use serde::Serialize;

use crate::Connect;
use crate::model::Me;

/// What the store sells, as StoreKit reports it (localized price included, never hard-coded).
#[derive(Debug, Clone, Serialize)]
pub struct StoreProduct {
    pub id: String,
    pub title: String,
    pub description: String,
    pub price: Option<String>,
    /// ISO 8601 length of one billing period (`P1M`, `P1Y`), shown to the user as the subscription length.
    pub period: Option<String>,
    /// Introductory offer (e.g. the yearly plan's one-month free trial), if the product has one.
    pub intro: Option<IntroOffer>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct IntroOffer {
    /// StoreKit's localized price of the introductory period ("Free" for a free trial).
    pub price: String,
    /// ISO 8601 length of one introductory period.
    pub period: String,
    /// How many introductory periods the offer lasts.
    pub periods: u32,
    pub free: bool,
}

/// One StoreKit pricing phase, reduced to what the subscription terms need.
#[cfg_attr(not(feature = "store"), allow(dead_code))]
pub(crate) struct Phase {
    /// `true` for the regular renewing price, `false` for an introductory phase.
    pub recurring: bool,
    pub price: String,
    pub period: String,
    pub cycles: i32,
}

/// A price string with no non-zero digit is free: "$0.00", "0,00 €", and the localized word for "Free".
fn is_free(price: &str) -> bool {
    !price.chars().any(|c| c.is_numeric() && c.to_digit(10).is_none_or(|d| d != 0))
}

/// Subscription length and introductory offer from a product's pricing phases.
#[cfg_attr(not(feature = "store"), allow(dead_code))]
pub(crate) fn subscription_terms(phases: &[Phase]) -> (Option<String>, Option<IntroOffer>) {
    let period = phases.iter().find(|p| p.recurring).map(|p| p.period.clone());
    let intro = phases.iter().find(|p| !p.recurring).map(|p| IntroOffer {
        free: is_free(&p.price),
        price: p.price.clone(),
        period: p.period.clone(),
        periods: p.cycles.max(1) as u32,
    });
    (period, intro)
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum SubscribeOutcome {
    /// Bought and linked; `me` carries the fresh entitlement.
    Linked { me: Me },
    /// Waiting for approval (Ask to Buy). The entitlement arrives once it is approved.
    Pending,
    Cancelled,
}

#[derive(Debug, Clone, Serialize)]
pub struct LinkOutcome {
    /// Subscription transactions sent to the server.
    pub linked: usize,
    pub me: Me,
}

#[cfg_attr(not(feature = "store"), allow(dead_code))]
/// Keeps the StoreKit product ids this app sells; anything else a caller asks for is refused.
pub(crate) fn is_known_product(products: &[String], id: &str) -> bool {
    products.iter().any(|p| p == id)
}

#[cfg_attr(not(feature = "store"), allow(dead_code))]
/// Plugin errors for the two non-error outcomes of a purchase sheet.
pub(crate) fn classify_purchase_error(message: &str) -> Option<PurchaseEnd> {
    let m = message.to_ascii_lowercase();
    if m.contains("cancelled") || m.contains("canceled") {
        Some(PurchaseEnd::Cancelled)
    } else if m.contains("pending") {
        Some(PurchaseEnd::Pending)
    } else {
        None
    }
}

#[cfg_attr(not(feature = "store"), allow(dead_code))]
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum PurchaseEnd {
    Cancelled,
    Pending,
}

#[cfg(not(feature = "store"))]
mod commands {
    use super::*;
    use tauri::State;

    const NOT_SOLD: &str = "Subscriptions are managed on your Voltrus account in this build.";

    #[tauri::command]
    pub async fn connect_store_products(_connect: State<'_, Connect>) -> Result<Vec<StoreProduct>, String> {
        Ok(Vec::new())
    }

    #[tauri::command]
    pub async fn connect_subscribe(_product_id: String, _connect: State<'_, Connect>) -> Result<SubscribeOutcome, String> {
        Err(NOT_SOLD.into())
    }

    #[tauri::command]
    pub async fn connect_link_purchases(_connect: State<'_, Connect>) -> Result<LinkOutcome, String> {
        Err(NOT_SOLD.into())
    }
}

#[cfg(feature = "store")]
mod commands {
    use super::*;
    use crate::account;
    use tauri::{AppHandle, Runtime, State};
    use tauri_plugin_iap::{IapExt, PurchaseOptions, PurchaseRequest, PurchaseStateValue};

    const SUBS: &str = "subs";

    #[tauri::command]
    pub async fn connect_store_products<R: Runtime>(app: AppHandle<R>, connect: State<'_, Connect>) -> Result<Vec<StoreProduct>, String> {
        if connect.config.subscription_products.is_empty() {
            return Ok(Vec::new());
        }
        // Products the store does not know yet (not approved, or sandbox not ready) are simply absent.
        let found = app
            .iap()
            .get_products(connect.config.subscription_products.clone(), SUBS.into())
            .await
            .map_err(|e| e.to_string())?;
        let mut out: Vec<StoreProduct> = found
            .products
            .into_iter()
            .map(|p| {
                let phases: Vec<Phase> = p
                    .subscription_offer_details
                    .iter()
                    .flatten()
                    .flat_map(|o| &o.pricing_phases)
                    .map(|ph| Phase {
                        recurring: ph.recurrence_mode == 1,
                        price: ph.formatted_price.clone(),
                        period: ph.billing_period.clone(),
                        cycles: ph.billing_cycle_count,
                    })
                    .collect();
                let (period, intro) = subscription_terms(&phases);
                StoreProduct { id: p.product_id, title: p.title, description: p.description, price: p.formatted_price, period, intro }
            })
            .collect();
        let order = |id: &str| connect.config.subscription_products.iter().position(|p| p == id).unwrap_or(usize::MAX);
        out.sort_by_key(|p| order(&p.id));
        Ok(out)
    }

    #[tauri::command]
    pub async fn connect_subscribe<R: Runtime>(
        app: AppHandle<R>,
        product_id: String,
        connect: State<'_, Connect>,
    ) -> Result<SubscribeOutcome, String> {
        if !is_known_product(&connect.config.subscription_products, &product_id) {
            return Err("Unknown subscription.".into());
        }
        // Bind the purchase to this account so the server can attribute renewals and refunds to it.
        let me = account::me(&connect.session).await.map_err(|e| e.to_string())?;
        let options = me.app_account_token.clone().map(|token| PurchaseOptions {
            offer_token: None,
            obfuscated_account_id: None,
            obfuscated_profile_id: None,
            app_account_token: Some(token),
            old_purchase_token: None,
            subscription_replacement_mode: None,
        });
        let bought = app
            .iap()
            .purchase(PurchaseRequest { product_id, product_type: SUBS.into(), options })
            .await;
        let purchase = match bought {
            Ok(p) => p,
            Err(e) => {
                return match classify_purchase_error(&e.to_string()) {
                    Some(PurchaseEnd::Cancelled) => Ok(SubscribeOutcome::Cancelled),
                    Some(PurchaseEnd::Pending) => Ok(SubscribeOutcome::Pending),
                    None => Err(e.to_string()),
                };
            }
        };
        if purchase.purchase_state == PurchaseStateValue::Pending {
            return Ok(SubscribeOutcome::Pending);
        }
        let jws = purchase.jws_representation.ok_or("The App Store did not return a signed transaction.")?;
        account::link_transaction(&connect.session, &jws).await.map_err(|e| e.to_string())?;
        let me = account::me(&connect.session).await.map_err(|e| e.to_string())?;
        Ok(SubscribeOutcome::Linked { me })
    }

    /// Sends the account's current App Store subscription transactions to the server. Cheap and
    /// idempotent: used for Restore, after sign-in, and on launch. Does not show any App Store sheet.
    #[tauri::command]
    pub async fn connect_link_purchases<R: Runtime>(app: AppHandle<R>, connect: State<'_, Connect>) -> Result<LinkOutcome, String> {
        let restored = app.iap().restore_purchases(SUBS.into()).await.map_err(|e| e.to_string())?;
        let mut linked = 0;
        let mut first_error = None;
        for p in restored.purchases {
            if !is_known_product(&connect.config.subscription_products, &p.product_id) || p.purchase_state != PurchaseStateValue::Purchased {
                continue;
            }
            let Some(jws) = p.jws_representation else { continue };
            match account::link_transaction(&connect.session, &jws).await {
                Ok(_) => linked += 1,
                Err(e) => first_error = first_error.or(Some(e.to_string())),
            }
        }
        if linked == 0 {
            if let Some(e) = first_error {
                return Err(e);
            }
        }
        let me = account::me(&connect.session).await.map_err(|e| e.to_string())?;
        Ok(LinkOutcome { linked, me })
    }
}

pub use commands::{connect_link_purchases, connect_store_products, connect_subscribe};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn purchase_sheet_outcomes_are_not_errors() {
        assert_eq!(classify_purchase_error("Purchase cancelled by user"), Some(PurchaseEnd::Cancelled));
        assert_eq!(classify_purchase_error("Purchase is pending"), Some(PurchaseEnd::Pending));
        assert_eq!(classify_purchase_error("Purchase failed: no network"), None);
    }

    fn phase(recurring: bool, price: &str, period: &str, cycles: i32) -> Phase {
        Phase { recurring, price: price.into(), period: period.into(), cycles }
    }

    #[test]
    fn monthly_has_a_length_and_no_intro() {
        let (period, intro) = subscription_terms(&[phase(true, "$4.99", "P1M", 0)]);
        assert_eq!(period.as_deref(), Some("P1M"));
        assert_eq!(intro, None);
    }

    #[test]
    fn yearly_free_trial_is_reported_as_intro() {
        let (period, intro) = subscription_terms(&[phase(false, "Free", "P1M", 1), phase(true, "$39.99", "P1Y", 0)]);
        assert_eq!(period.as_deref(), Some("P1Y"));
        assert_eq!(intro, Some(IntroOffer { price: "Free".into(), period: "P1M".into(), periods: 1, free: true }));
    }

    #[test]
    fn paid_intro_is_not_free() {
        for price in ["$0.99", "0,99 €", "¥150"] {
            assert!(!is_free(price), "{price}");
        }
        for price in ["Free", "$0.00", "0,00 €", "Gratis"] {
            assert!(is_free(price), "{price}");
        }
    }

    #[test]
    fn only_configured_products_can_be_bought() {
        let products = vec!["a.connect.monthly".to_string(), "a.connect.yearly".to_string()];
        assert!(is_known_product(&products, "a.connect.yearly"));
        assert!(!is_known_product(&products, "a.pro"));
    }
}

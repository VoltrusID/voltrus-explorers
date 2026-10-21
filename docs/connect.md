# Voltrus Connect in the explorer apps

Connect is an optional Voltrus account: saved profiles sync across devices, profiles can be shared with a team, and
Voltrus Remote sites are listed with their join steps. Nothing in the apps needs it. The server side and the API
contract live in the `voltrus-console` repository (`/api/connect/*`, `src/lib/connect/clients.ts`).

## Pieces

- `crates/mt-connect`: sign-in (OAuth 2.0 authorization code + PKCE in the system browser, back through the
  `voltrus-<slug>://connect` deep link), tokens in the macOS/iOS Keychain, token refresh, profile sync, teams, Remote
  sites, and (feature `store`) the App Store subscription. Apps call `mt_connect::setup(builder, include_str!("../connect.json"))`
  after `mt_store::setup` and register the `connect_*` commands.
- `packages/mt-ui/src/connect.tsx`: `ConnectProvider`, `ConnectButton` (toolbar), the Connect dialog, and
  `useConnectApp({ storage, subscribe, captureCurrent, open })` which each app uses to hand over its profiles.
  `connectSync.ts` holds the pure sync bookkeeping.
- Per app: `src-tauri/connect.json` and the `plugins.deep-link` block in `tauri.conf.json`.

```json
{
  "app_slug": "modbus-poll",
  "subscription_products": ["com.joyodigitama.modbuspoll.connect.monthly", "com.joyodigitama.modbuspoll.connect.yearly"]
}
```

`app_slug` is the OAuth `client_id`, the profile `app` value and the URL scheme suffix. It must be registered in the
console (`src/lib/connect/clients.ts`) with the redirect `voltrus-<slug>://connect`, and the product ids must match the
console's per-app table.

## What is synced

| App | Profile |
| --- | --- |
| Modbus Poll | A saved connection: transport, TCP and serial settings, read blocks. Key = profile name. |
| BACnet Explorer | A device (IP, port, device id) and its watch list (objects, poll or COV). Saved from the Connect dialog. |

Profile data is stored by the server as plain JSON: never put passwords or tokens in it.

## Behaviour worth knowing

- Sync is a pass over the app's profiles: upload what changed, pull what is newer, add what is new, remove what other
  devices deleted (tombstones only). A profile that disappears from a list (team profile while the subscription is
  inactive) is never deleted locally. Deleting a profile on this device asks before it is removed from the account.
- Both sides changed: the user keeps this device's version or takes the account's. The server returns 409 with its
  copy; nothing is overwritten silently.
- Refresh tokens rotate and are single use. The new one is written to the Keychain before the access token is used;
  a rejected refresh signs the user out, a network failure does not.
- Outside the App Store the entitlement comes from the console; the dialog only offers a link to the account page.
  In App Store builds the dialog shows the StoreKit product titles and localized prices, Restore, the renewal
  disclosure and Terms/Privacy links. It never links to console pricing.
- Until StoreKit returns Connect products (they have no price yet) an App Store build shows no subscribe panel and no
  text about a paid plan (`plansOffered` in `connect.tsx`); sign-in and profile sync work as a free feature.
- Account > Delete account opens `<console>/app/settings?delete=1` (`ConnectInfo.delete_account_url`). The console confirms
  with an emailed code, erases the Connect data and anonymizes the account (App Store guideline 5.1.1(v)).

## App Store Connect checklist (per app)

1. Create an auto-renewable subscription group with the two products from `connect.json` (monthly, yearly). Product
   ids are final once created. Keep them inactive until the price is decided.
2. Point App Store Server Notifications V2 (production and sandbox) at `https://console.voltrus.id/api/connect/app-store/notifications`.
3. Once the products are priced, attach the subscriptions to the app version that ships Connect, with a review screenshot of the Connect dialog.
4. Privacy Policy: the dialog links `https://voltrus.id/privacy`; the page must exist before review.

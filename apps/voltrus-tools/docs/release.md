# Releasing Voltrus Tools to the Mac App Store

Voltrus Tools ships in two flavours from the same source:

| Build | How | Licence |
| --- | --- | --- |
| Source, CI (`voltrus-tools.yml`), GitHub | `cargo build -p vt-app` | everything unlocked, nothing sold ([`Licence::Unrestricted`](../crates/vt-store/src/lib.rs)) |
| Mac App Store | `cargo build -p vt-app --features store` ([`scripts/build-mas.sh`](../scripts/build-mas.sh)) | free download, 14-day trial, then a one-time unlock |

The individual explorer apps stay on sale beside it; their pipeline is [`docs/release.md`](../../../docs/release.md).

## Licensing

### Products

| Product id | Type | Price | App Store name |
| --- | --- | --- | --- |
| `com.joyodigitama.voltrustools.trial14` | non-consumable | USD 0 | 14-day Trial |
| `com.joyodigitama.voltrustools.unlock` | non-consumable | USD 49 | Voltrus Tools |

The trial follows App Review guideline
[3.1.1](https://developer.apple.com/app-store/review/guidelines/#in-app-purchase) to the letter:

> Non-subscription apps may offer a free time-based trial period before presenting a full unlock option by setting up a
> Non-Consumable IAP item at Price Tier 0 that follows the naming convention: "XX-day Trial." Prior to the start of the
> trial, your app must clearly identify its duration, the content or services that will no longer be accessible when the
> trial ends, and any downstream charges the user would need to pay for full functionality.

So the trial starts when the user buys the free "14-day Trial" item, and its start is that item's
`originalPurchaseDate` (the first purchase on the Apple Account; a restore or re-download keeps it). The download date
(`AppTransaction.originalPurchaseDate`) is not used: the guideline has the user start the trial after the app has
shown its terms, which a download can't do. The unlock sheet shown before the trial states the 14 days, that
connecting, simulating and writing stop afterwards (saved data stays readable), and the unlock's price from the App
Store.

### States

| State | When | The app |
| --- | --- | --- |
| Not started | no trial item, no unlock | everything opens; connecting, simulators, writes and demos open the unlock sheet; status bar: "Start free trial" |
| Trial | trial item bought less than 14 days ago | everything works; status bar: "Trial · N days left" (rounded up, amber from 3) |
| Expired | 14 × 24 h after the trial's start | read-only: saved connections, layouts, points, trends and logs open; connecting, simulating and writing are refused with the unlock sheet; running sessions stop at the moment it expires; status bar: "Read-only · Unlock" |
| Unlocked | unlock bought | everything, for good |

The logic is pure Rust in [`vt-store`](../crates/vt-store/src/lib.rs) (`Licence::evaluate`, `Cache`) with unit tests for
day rounding, the expiry boundary, clock rollback, restore, refunds and Apple Account switches.

- **The gate.** The model talks to the engine only through `GatedEngine`
  ([`model/licence.rs`](../crates/vt-app/src/model/licence.rs)). Unless the licence allows work, it passes only `Close`
  and `Shutdown`; every other command (open a session, write, hold a value, publish, relinquish, discover, and any
  protocol command added later) is dropped and the unlock sheet opens. `connect`, the demos and BACnet discovery check
  first, so nothing is half started. Tests: `an_expired_trial_refuses_connecting_and_asks_to_unlock`,
  `the_gated_engine_drops_refused_commands`; the `licence-gate` screenshot shows it in the app.
- **Verified, cached, offline.** StoreKit 2 checks each transaction's App Store signature on the device; only verified,
  unrevoked transactions count. The last answer is saved as `licence.json` beside `workspace.json` (inside the
  container), so a buyer is unlocked at launch, offline included. When StoreKit can't answer, the cache stays.
- **Clock.** The cache keeps the latest time the Mac has seen (its clock, and the App Store's signature times). Turning
  the clock back stops at that time: it neither extends the trial nor locks anyone out. A clock behind the purchase
  counts as the moment of purchase (never more than 14 days).
- **Restore** (unlock sheet and About) runs `AppStore.sync()` and takes the answer. A refund removes the unlock at the
  next StoreKit answer. A trial started on this Mac stays started from its earliest date, even under another Apple
  Account.
- **Updates.** `Transaction.updates` (Ask to Buy approvals, purchases on another Mac, refunds) refresh the licence; the
  trial is re-checked every minute.

### StoreKit bridge

GPUI isn't Tauri, so `mt-store`'s Tauri plugin doesn't fit. [`swift/StoreKitBridge.swift`](../crates/vt-store/swift/StoreKitBridge.swift)
wraps StoreKit 2 in five C functions (snapshot, purchase, restore, listen, free) that answer JSON. With the `store`
feature, [`vt-store/build.rs`](../crates/vt-store/build.rs) compiles it with `swiftc` into a static library per target
architecture and links it against the Swift runtime in macOS (deployment target 13.0, set for every build in
[`.cargo/config.toml`](../.cargo/config.toml): below 12.0 the Swift concurrency runtime would be linked by `@rpath`
and the app wouldn't launch). Without the feature nothing
Swift is built and every store call fails. Calls block, so the app makes them on their own thread.

### Screens

- Unlock sheet: opens from the status bar, the menu (Voltrus Tools › Unlock Voltrus Tools…) or a refused command.
  Enter runs the main button (start trial before the trial, unlock after), Esc closes.
- About sheet (Voltrus Tools › About Voltrus Tools): version, flavour, licence state, Restore purchases, the GPL notice
  and where the third-party licences are. Store builds show no link to the GitHub builds (guideline 3.1.1: no other
  way to buy or get the unlock); source builds do.
- Source builds preview every state without buying anything: `voltrus-tools --licence-preview
  not-started|trial|expired|unlocked|gate`. Store builds ignore the flag. CI captures each as
  `licence-<state>.png` in the `voltrus-tools-screenshots` artifact.

## Sandbox

[`packaging/entitlements.mas.plist`](../packaging/entitlements.mas.plist), plus the application and team identifiers
from the provisioning profile:

| Entitlement | Why |
| --- | --- |
| `app-sandbox` | required on the Mac App Store |
| `network.client` | every client: Modbus TCP, MQTT, BACnet/IP, OPC UA, S7, EtherNet/IP, DNP3, NATS |
| `network.server` | the simulators: Modbus slave, MQTT broker, BACnet device, OPC UA server, DNP3 outstation; listeners open only when the user starts one. BACnet and EtherNet/IP discovery also receive broadcasts |
| `files.user-selected.read-write` | importing and exporting through open/save panels (e.g. OPC UA user certificates, serial logs) |
| `device.serial` | Modbus RTU and the serial terminal |
| `device.bluetooth` | the BLE inspector; `NSBluetoothAlwaysUsageDescription` is set by `build-mas.sh` |

`NSLocalNetworkUsageDescription` is in [`packaging/Info.plist`](../packaging/Info.plist). The protocol agents' reports
for this release: OPC UA keeps its PKI under the data folder and needs nothing more; the serial simulator's pty and its
symlink live in the container's temporary folder.

Storage stays inside the container: `vt_app_core::store::default_path()` comes from `$HOME`, which the sandbox points
at `~/Library/Containers/com.joyodigitama.voltrus-tools/Data`, and everything the app keeps (workspace, licence
cache, certificates) goes beside it. Temporary files use `std::env::temp_dir()` (the container's `TMPDIR`).

## Building and uploading

**Universal** (arm64 + x86_64), like every Mac App Store build in this repository: macOS 13, the minimum, still runs
on Intel Macs, and the App Store serves one binary to both. CI's plain build stays arm64 because it only feeds
screenshots.

```bash
scripts/build-mas.sh bundle dist-store                         # cargo build --features store for both targets, lipo, bundle
MAS_PROFILE=… scripts/build-mas.sh package "dist-store/Voltrus Tools.app" dist-store [--upload]
```

`bundle` sets the bundle id `com.joyodigitama.voltrus-tools` (source builds keep `id.voltrus.tools`), the build number
(Unix time) and the Bluetooth usage string. `package` embeds the profile, signs with Apple Distribution and the sandbox
entitlements (hardened runtime, timestamp), packs `VoltrusTools-<version>.pkg` with the Mac Installer identity and
validates it with App Store Connect, or uploads it with `--upload`. No notarization: the App Store re-signs.

### Release

1. Bump `version` in `apps/voltrus-tools/Cargo.toml` (`[workspace.package]`) to the App Store version (1.0.0 for the
   first release; App Store Connect has version 1.0.0 waiting) and merge to `main`.
2. Run from `main` (the `release` environment holds the signing secrets and deploys from `main` only):

   ```bash
   gh workflow run release.yml -R VoltrusID/voltrus-explorers --ref main \
     -f app=voltrus-tools -f appstore=true -f dmg=false -f upload=true
   ```

   The `voltrus-tools` job builds the bundle, fetches or creates the `voltrus-ci MAC_APP_STORE` profile through
   `scripts/asc-profile.py` and packages and uploads the `.pkg`.
3. In App Store Connect, answer export compliance for the build, select it on the version, add both in-app purchases
   to the version, and submit them together (Apple requires a first non-consumable to ship with an app version).

To upload from a branch, which the `release` environment refuses: download the `voltrus-tools-mas-unsigned` artifact
of a `voltrus-tools.yml` run, then on a Mac with both identities and the API key:

```bash
python3 scripts/asc-profile.py MAC_APP_STORE com.joyodigitama.voltrus-tools <Apple Distribution serial> mas.provisionprofile
MAS_PROFILE=mas.provisionprofile ASC_KEY_ID=… ASC_ISSUER_ID=… \
  apps/voltrus-tools/scripts/build-mas.sh package "Voltrus Tools.app" out --upload
```

Test purchases in TestFlight builds use the sandbox, so nothing is charged.

## App Store Connect

| Object | Id |
| --- | --- |
| App "Voltrus Tools" (macOS, SKU `voltrus-tools`) | `6827042077` |
| Bundle id `com.joyodigitama.voltrus-tools` | `R2K54Y3BL6` |
| Version 1.0.0 (Prepare for Submission) | `2d83afe3-a5db-4217-ac85-7d485056752e` |
| IAP Unlock, USD 49.00, all territories | `6827042132` |
| IAP 14-day Trial, USD 0.00, all territories | `6827042266` |

- Price: free. Category: Developer Tools, secondary Utilities, like BACnet Explorer and EtherNet/IP Explorer. The
  people who buy it commission and debug PLCs, meters and controllers, the same audience as the explorer apps, and the
  App Store lists protocol analysers there.
- Age rating 4+. App Privacy: "Data Not Collected" (true: no account, analytics, telemetry or crash reporting;
  StoreKit is Apple's).
- Listing (en-US): subtitle "Modbus, MQTT, OPC UA & BACnet"; promotional text "Poll, simulate and trend Modbus, MQTT,
  BACnet, OPC UA, S7, EtherNet/IP and DNP3 devices from one Mac app. Every tool is free to try for 14 days."; keywords
  `plc,scada,s7,dnp3,ethernetip,sparkplug,nats,serial,rs485,ble,simulator,hmi,iot,automation,industrial`; the
  description covers each protocol, the views, the trial and unlock, and privacy. Re-read it against the protocols
  that actually ship before submitting. Marketing, support and privacy URLs: `https://voltrus.id/voltrus-tools/`,
  `…/support`, `…/privacy`.

### Before the first submission

1. Publish `https://voltrus.id/voltrus-tools/`, `/support` and `/privacy` (404 on 2026-10-26).
2. Review screenshots for both in-app purchases (the unlock sheet, `licence-not-started.png` and
   `licence-expired.png`, at 1280×800 or larger) and the app screenshots (1280×800, 1440×900, 2560×1600 or 2880×1800).
3. App Review information: untick "Sign-in required", add contact details, note that the trial starts from the status
   bar or by connecting anything.
4. Content Rights declaration; publish the App Privacy answer.

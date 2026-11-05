# Releasing Voltrus Tools

Voltrus Tools ships in three flavours from the same source:

| Build | How | Licence |
| --- | --- | --- |
| Source, CI (`voltrus-tools.yml`), GitHub | `cargo build -p vt-app` | everything unlocked, nothing sold ([`Licence::Unrestricted`](../crates/vt-store/src/lib.rs)) |
| Mac App Store | `cargo build -p vt-app --features store` ([`scripts/build-mas.sh`](../scripts/build-mas.sh)) | free download, 14-day trial, then a one-time unlock |
| Direct download (DMG) | `cargo build -p vt-app --features license` ([`scripts/build-dmg.sh`](../scripts/build-dmg.sh)) | free download, 14-day trial, then a one-time licence key (USD 49) from the Voltrus console; see [Direct download](#direct-download-licence-key) |

`store` and `license` are exclusive: enabling both is a compile error. App Review rejected the App Store build under
guideline 4.3, so the direct download is the one on sale; the App Store sections below stay for the record and for a
later resubmission.

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
  ([`model/licence.rs`](../crates/vt-app/src/model/licence.rs)). Unless the licence allows work, it passes only `Close`,
  `Shutdown` and `ListSerialPorts` (a local read for the editor, since read-only mode still edits saved connections);
  every other command (open a session, write, hold a value, publish, relinquish, discover, serial send or log, and any
  protocol command added later) is dropped and the unlock sheet opens. `connect`, every demo and BACnet and
  EtherNet/IP discovery check first, so nothing is half started. Tests:
  `an_expired_trial_refuses_connecting_and_asks_to_unlock`, `the_gated_engine_drops_refused_commands`,
  `protocol_commands_are_refused_once_the_trial_ends` and `every_protocol_and_role_is_refused_once_the_trial_ends`,
  which opens a session of every `Protocol::ALL` entry as client and simulator (its `spec_for` match has no wildcard,
  so a new protocol doesn't compile until it is covered); the `licence-gate` screenshot shows it in the app. When a
  protocol lands, start its demo and discovery with `if !self.licence_allows(cx) { return …; }` like the others.
- **Verified, cached, offline.** StoreKit 2 checks each transaction's App Store signature on the device; only verified,
  unrevoked transactions count. The last answer is saved as `licence.json` beside `workspace.json` (inside the
  container), so a buyer is unlocked at launch, offline included. When StoreKit can't answer, the cache stays.
- **Clock.** The cache keeps the latest time the Mac has seen (its clock, and the App Store's signature times). Turning
  the clock back stops at that time: it neither extends the trial nor locks anyone out. A clock behind the purchase
  counts as the moment of purchase (never more than 14 days). That time and the trial's earliest start are also kept
  in the Keychain (`SystemMarks`), and the later time and earlier start win, so deleting `licence.json` and turning
  the clock back doesn't reset the trial either.
- **An edited cache.** A trial start StoreKit doesn't confirm only ever means the trial was used, never a running
  trial: at launch a cached start is never later than the latest time seen, and when StoreKit answers without the
  trial item, the cached start is moved at least 14 days back. Tests:
  `a_trial_storekit_does_not_confirm_does_not_run` and
  `deleting_the_cache_and_turning_the_clock_back_does_not_extend_the_trial`.
- **Restore** (unlock sheet and About) runs `AppStore.sync()` and takes the answer. A refund removes the unlock at the
  next StoreKit answer. A trial started on this Mac stays used, even under another Apple Account.
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
| `network.client` | every client: Modbus TCP, MQTT, BACnet/IP, OPC UA, S7, EtherNet/IP, DNP3, NATS, MTConnect (HTTP), FANUC FOCAS/Ethernet, SECS/GEM (HSMS) |
| `network.server` | the simulators: Modbus slave, MQTT broker, BACnet device, OPC UA server, DNP3 outstation, MTConnect agent (and its SHDR adapter), FANUC CNC, SECS/GEM equipment (HSMS); listeners open only when the user starts one. BACnet and EtherNet/IP discovery also receive broadcasts |
| `files.user-selected.read-write` | importing and exporting through open/save panels (e.g. OPC UA user certificates, serial logs) |
| `device.serial` | Modbus RTU and the serial terminal |
| `device.bluetooth` | the BLE inspector; `NSBluetoothAlwaysUsageDescription` is set by `build-mas.sh` |

App Review's automated scan flagged `network.server` on 1.0.0 as "no matching functionality". Keep the explanation of
the simulators' listeners (which demo, which port) at the top of the App Review Information notes.

App Review also rejects private API symbols found in the binary. gpui's `CGSSetWindowBackgroundBlurRadius` (macOS < 12 blur)
is removed in the pinned fork commit; check a new gpui pin with
`llvm-nm -u "Voltrus Tools.app/Contents/MacOS/voltrus-tools" | grep -E '_CGS|_SLS'` (expect no output).

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
  NATS, BACnet, OPC UA, S7, EtherNet/IP, DNP3, serial and BLE devices from one Mac app. Every tool is free to try for
  14 days."; keywords
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

## Direct download (licence key)

A notarized Developer ID DMG, `Voltrus-Tools-<version>-universal.dmg`, served from
`https://dl-voltrus-tools.voltrus.id/latest` (the worker serves only files matching `/voltrus[ _.-]*tools/i`). It is
the same app as the store build with the `license` feature instead of `store`: no StoreKit, no sandbox.

### States

The same four states, the same gate and the same screens as the [store build](#states); only where the trial and the
unlock come from differs.

| State | When | Comes from |
| --- | --- | --- |
| Not started | no trial start known on this Mac, no key | the unlock sheet offers "Start free trial", "Buy licence · $49" and the key field |
| Trial | started less than 14 days ago | "Start free trial" in the unlock sheet (which the status bar, the menu, the first connect or demo open) |
| Expired | 14 × 24 h after the start | read-only, exactly like the store build |
| Unlocked | a licence key is activated on this Mac | the key field's Activate |

- **The trial without StoreKit.** Starting it writes its start (`Cache::start_trial`) to `licence.json` and to the
  Keychain marks (`SystemMarks`, item `com.joyodigitama.voltrus-tools.licence` / `marks`, now kept in `license`
  builds too). The earlier start and the later clock floor win, so deleting `licence.json`, rewriting it or turning
  the clock back neither restarts nor extends the trial; asking for a new trial keeps the first start. Tests in
  [`vt-store`](../crates/vt-store/src/lib.rs): `a_trial_started_on_this_mac_runs_fourteen_days_then_turns_read_only`,
  `starting_the_trial_again_never_restarts_it`, `the_keychain_marks_outlive_a_deleted_cache_without_storekit`,
  `a_clock_turned_back_before_starting_counts_from_the_latest_time_seen`.
- **The unlock is the key in the Keychain**, not the cache file: at launch a saved key unlocks at once (offline
  included), then it is validated once in the background.
- **Unlock sheet**: a "Licence key" field with Activate (Enter in the field activates; Enter on the sheet activates a
  typed key, otherwise starts the trial or opens the buy page), "Buy licence · $49" (opens `buy_url` in the browser),
  and, when the server answers `code: "seat_limit"`, "Move licence here" (asks once more, then calls `/license/move`).
  Once unlocked, the unlock sheet and About show "Remove key" (asks once more), which releases this Mac's seat and
  forgets the key; the app goes back to the trial state it had (expired, as a rule). No "Restore purchases".
- **About** says "direct download" and, unlike the store build, links to the source on GitHub.

### Licence server protocol

[`vt-store/src/license.rs`](../crates/vt-store/src/license.rs) mirrors `mt-store`'s `license` feature (the
explorer apps' licence-key DMGs), field for field:

```text
POST {server}/license/activate   { key, device_id, app_version, platform } -> { valid, activation_id?, error? }
POST {server}/license/validate   { key, activation_id, device_id, app_version, platform } -> { valid, error? }
POST {server}/license/move       { key, device_id, app_version, platform } -> { valid, activation_id?, error?, code? }
POST {server}/license/deactivate { key, activation_id, device_id }       -> { ok }
```

`platform` is `macos-aarch64` or `macos-x86_64`. As in `mt-store`, the bodies carry no app name: the worker serves
one app, so `dl-voltrus-tools` is the app. The rules:

- Validated once per launch, in the background. Only an explicit `{"valid": false}` removes the key (with a toast);
  an unreachable server, a 5xx or an answer that isn't the worker's JSON keeps the unlock, so offline buyers are
  never locked out.
- Activation needs an `activation_id` in a valid answer; without one nothing is saved. Polar's raw error JSON is
  shown by its `detail`; an unknown key reads "That licence key was not recognised."; the `seat_limit` text is shown
  whole and offers "Move licence here".
- Remove calls `/license/deactivate` best effort: the key is forgotten even offline.
- The key and its activation id (item `licence`) and a per-device id made once (item `device-id`, kept when the key
  is removed so the Mac doesn't take a new seat) are generic passwords of service `com.joyodigitama.voltrus-tools`
  in the login Keychain; on other systems (tests) they are files in `licence-key/` beside `licence.json`.

Tests run the real reqwest path against a mock HTTP server: valid, refused, unknown key, seat limit then move,
remove (online and offline), unreachable server, 5xx and garbage answers keep the unlock, explicit invalid removes
it, once per launch, and the blocking `Keys` front the app calls from its background thread.

### Configuration

[`vt-store/store.json`](../crates/vt-store/store.json), the `license` section of the explorer apps' `store.json`,
compiled into the app:

```json
{ "license": { "server": "https://dl-voltrus-tools.voltrus.id", "buy_url": "https://console.voltrus.id/checkout/p/6d83af93-bc3a-41dc-af90-51a907270790", "price": "$49" } }
```

The Polar product is `6d83af93-bc3a-41dc-af90-51a907270790` (USD 49, licence-key benefit `VOLTRUS-TOOLS`
`4f7e744a-77ed-47c2-babc-31b5dde7379b`, one activation, movable); the console's `VoltrusToolsLicence` accepts only that
benefit and `VLT-` licences of the `voltrus-tools` app.

### Sandbox and signing

No App Sandbox: hardened runtime with [`packaging/entitlements.dmg.plist`](../packaging/entitlements.dmg.plist),
which is empty. Network, serial and Bluetooth need no entitlement outside the sandbox; macOS asks the user through
`NSLocalNetworkUsageDescription` and `NSBluetoothAlwaysUsageDescription`, both in
[`packaging/Info.plist`](../packaging/Info.plist) (`build-dmg.sh` checks them). The bundle id is the store's,
`com.joyodigitama.voltrus-tools`. Data lives in the real home folder (`vt_app_core::store::default_path()` from
`$HOME`), not in a container, so a Mac that ran the store build keeps two separate workspaces.

```bash
scripts/build-dmg.sh bundle dist-dmg                                        # cargo build --features license for both targets, lipo, bundle
ASC_KEY_ID=… ASC_ISSUER_ID=… ASC_KEY_PATH=… \
  scripts/build-dmg.sh package "dist-dmg/Voltrus Tools.app" dist-dmg --notarize   # Developer ID, DMG, notarize, staple
```

`package` signs the app (Developer ID Application, hardened runtime, timestamp), packs it with an Applications link
into `Voltrus-Tools-<version>-universal.dmg`, signs the DMG, submits it to `notarytool` with the App Store Connect
API key, staples it and checks it with `spctl`.

### Release

1. Bump `version` in `apps/voltrus-tools/Cargo.toml` (`[workspace.package]`) and merge to `main`.
2. Build, sign, notarize and upload the DMG as a run artifact:

   ```bash
   gh workflow run release.yml -R VoltrusID/voltrus-explorers --ref main \
     -f app=voltrus-tools -f appstore=false -f dmg=true
   ```

   (`appstore` defaults to true; leave it on to build the store `.pkg` in the same run.) The `voltrus-tools` job
   bundles before any secret is on the runner, then imports the `DEVELOPER_ID_*` identity with
   `scripts/ci-keychain.sh` and runs `build-dmg.sh package --notarize`. `github_release=true` is refused: the DMG is
   sold, not put on GitHub. The run summary prints the publish command.
3. Publish it to the download worker, locally with your Cloudflare login:

   ```bash
   scripts/publish-direct.sh voltrus-tools <version> --run <run id>
   ```

   That script's `voltrus-tools` entry and the `dl-voltrus-tools` worker (bucket, `/latest`, the `/license/*`
   routes against the Polar product) are set up separately, outside this app.

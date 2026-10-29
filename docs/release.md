# Releases

All builds run on GitHub-hosted `macos-15` runners. No local Mac is needed.

| Workflow | Trigger | Secrets | Does |
| --- | --- | --- | --- |
| `ci.yml` | push to `main`, pull requests, manual | none | crate tests, frontend typecheck/build/tests (Linux); unsigned universal build of both apps (macOS) |
| `release.yml` | manual only | environment `release` | Mac App Store `.pkg`, notarized Developer ID `.dmg`, iOS IPA of the apps that ship on iOS (Modbus Poll, BACnet Explorer, S7 Explorer, OPC UA Explorer; iPhone and iPad); optional App Store Connect upload and GitHub Release (never for Serial Monitor, BLE Inspector and CNC Explorer, see below) |

Voltrus Tools (the GPUI suite in `apps/voltrus-tools`) has its own Mac App Store path: `release.yml` with
`app=voltrus-tools`, a 14-day trial and a one-time unlock through StoreKit 2. See
[apps/voltrus-tools/docs/release.md](../apps/voltrus-tools/docs/release.md).

## Running a release

1. Bump `version` in `apps/<app>/src-tauri/tauri.conf.json` and `apps/<app>/package.json`, merge to `main`.
2. Run:

   ```bash
   gh workflow run release.yml -R VoltrusID/voltrus-explorers -f app=bacnet-explorer \
     -f appstore=true -f dmg=true -f ios=true -f upload=true -f github_release=true
   gh workflow run release.yml -R VoltrusID/voltrus-explorers -f app=modbus-poll \
     -f appstore=true -f dmg=true -f ios=true -f upload=true -f github_release=true
   ```

3. In App Store Connect, pick the uploaded build for the new version and submit it for review.

Build numbers (`CFBundleVersion`) are the Unix time of the build, the scheme the earlier App Store builds used, so they
always increase.

## What the release job does

- `scripts/ci-keychain.sh` writes the App Store Connect key to `~/.appstoreconnect/private_keys/` and imports the three
  signing identities into a temporary keychain; the last step deletes both.
- `scripts/asc-profile.py` gets (or creates) the provisioning profile `voltrus-ci <type> <bundle id> <cert id>` through
  the App Store Connect API, so profiles never need downloading by hand.
- `scripts/build-macos.sh <app> appstore|dmg|unsigned` builds the universal `.app` with Tauri, then signs it:
  - `appstore`: builds with `--features store` (Pro sold through StoreKit, see below), embeds the profile, signs with
    Apple Distribution and entitlements from `src-tauri/entitlements.plist` plus the profile's application/team
    identifier, packs a `.pkg` signed with the Mac Installer identity, then validates it against App Store Connect
    (or uploads it with `upload=true`).
  - `dmg`: signs with Developer ID (hardened runtime), packs and signs the DMG, notarizes it with `notarytool`
    (API key) and staples it. Apps with a `license` feature in `src-tauri/Cargo.toml` (Serial Monitor, BLE Inspector, CNC Explorer)
    are built with `--features license` (Pro by licence key, see below); every other app's DMG has everything unlocked.
- iOS: `apps/<app>/scripts/build-ios.sh` (always `--features store`) for the selected app, with Tauri's manual signing
  (`IOS_CERTIFICATE`, `IOS_CERTIFICATE_PASSWORD`, `IOS_MOBILE_PROVISION`). The profile is made for the bundle id in the
  app's `tauri.conf.json`. CI (`ci.yml`) also runs `cargo check --target aarch64-apple-ios` for every app that ships on iOS (Modbus Poll, BACnet Explorer, S7 Explorer, OPC UA Explorer).
- BACnet Explorer on iOS needs Apple's multicast entitlement for Who-Is scanning: see [ios-multicast.md](ios-multicast.md).
  Until it is granted the build simply leaves it out; connecting by IP address works either way.

## File access in the sandbox

The App Store builds run in the App Sandbox, where `<input type=file>` and `<a download>` blobs reach nothing. All CSV
import/export (read blocks, comm log, traffic, BACnet trend logs) goes through `@mactools/ui`'s `saveTextFile` /
`openTextFile`: a native save/open panel (`tauri-plugin-dialog`) grants access to exactly the path the user picks and
`tauri-plugin-fs` reads or writes it. Both apps therefore carry
`com.apple.security.files.user-selected.read-write` in `src-tauri/entitlements.plist`, register both plugins in their
Rust builder, and allow `dialog:allow-open|save` and `fs:allow-read|write-text-file` in `src-tauri/capabilities/`
(the fs permissions have no static scope: only dialog-picked paths are reachable). UI only reports "Saved" after the
write succeeded.

## OPC UA Explorer: sandbox notes

The OPC UA client and the built-in server keep their certificates and keys under `<app data dir>/pki/` (`client/`,
`server-<port>/`), set once at startup with `mt_opcua::pki::set_root`. In the App Store build the app data directory is
inside the container, so the folder is writable and survives restarts (the old build used the temp directory, which
the system clears). The built-in server needs `com.apple.security.network.server`; keep it and, if App Review's
scanner asks, answer with the justification already in the review notes (the server binds a TCP listener only when
the user starts server mode).

## Pro (in-app purchase and licence key)

**Serial Monitor, BLE Inspector and CNC Explorer are distributed through the Voltrus console and landing page only.**
Apple rejected Serial Monitor and BLE Inspector under guideline 4.3 (Spam) and those App Store submissions were withdrawn;
CNC Explorer is sold direct from the start. They are not on GitHub Releases either:
the notarized DMG is free for anyone to download, without signing in, from `https://dl-serial-monitor.voltrus.id/latest`
`https://dl-ble-inspector.voltrus.id/latest` and `https://dl-cnc-explorer.voltrus.id/latest` (linked from voltrus.id and console.voltrus.id), and Pro is a licence
key sold through Polar in the console (one-time, USD 4.99 each for Serial Monitor and BLE Inspector, USD 39.99 for CNC Explorer, one Mac; Pro features as in the README; CNC Explorer's Polar product is `6f42df6e-6dd2-46f1-af23-72d2e074534c`, keys `VOLTRUS-CNC-...`). The `store.json` files
keep the App Store product ids below, unused; do not dispatch `appstore=true` for any of them, and do not publish their
DMGs on GitHub (`release.yml` refuses `github_release=true` for them).

Release one of them:

1. Bump the version as above and merge to `main`.
2. Build the notarized DMG (built with `--features license`; `github_release` must stay off):

   ```bash
   gh workflow run release.yml -R VoltrusID/voltrus-explorers --ref main -f app=serial-monitor \
     -f appstore=false -f dmg=true -f ios=false -f upload=false -f github_release=false
   ```

3. Publish it to the app's download worker, locally with your own Cloudflare login (`npx wrangler login`) and `gh`:

   ```bash
   scripts/publish-direct.sh serial-monitor 1.6.0 --run <workflow run id>     # or a local path to the .dmg
   scripts/publish-direct.sh ble-inspector 1.3.0 --run <workflow run id>
   scripts/publish-direct.sh cnc-explorer 1.0.0 --run <workflow run id>
   ```

   The script downloads the run artifact, checks the notarization (on macOS), uploads the DMG to the R2 bucket
   (`serial-monitor-releases`, `ble-inspector-releases`, `cnc-explorer-releases`) and writes the `releases` and `release_files`
   (`darwin-aarch64`, `darwin-x86_64`) rows to D1 (`serial-monitor-releases-db`, `ble-inspector-releases-db`, `cnc-explorer-releases-db`), then
   checks that `/latest` redirects to the new file. `--dry-run` prints what it would do. The run id is in the
   workflow summary and the run's URL.

### Pro by licence key (`license` feature)

- `crates/mt-store` with `--features license` (exclusive with `store` and `keys`): Pro is a licence key activated against the
  app's download worker, the same `POST /license/activate` and `/license/validate` service the older MacTools builds
  used (`license` section of `src-tauri/store.json`: `server`, `buy_url`, `price`). Keys are Polar licence keys or
  console `VLT-XXXXX-XXXXX-XXXXX` licences (app slugs `serial-terminal`, `ble-inspector`, `mactools`; CNC Explorer uses Polar keys `VOLTRUS-CNC-...`).
- The key, its activation id and a per-device id are kept in the macOS Keychain (the app's own generic-password items,
  sandbox-safe); other platforms use files under the app data directory. The key is validated once per launch.
  An unreachable server, a 5xx or an unreadable answer keeps Pro (a paying user is never locked out offline); only
  an explicit `{"valid": false}` removes the key and locks Pro.
- Frontend: the same key flow as the Windows build (`keys`, see Windows below). `store_status` reports `activation: true`,
  `buy_url`, `price` and `reason: "key"` once unlocked, but `store: false`: a DMG is not an App Store build, so the virtual
  port and the GitHub source link stay. `store_activate(key, relocate?)` and `store_deactivate()` return the new
  status. `ProProvider` (`packages/mt-ui/src/pro.tsx`) shows Buy Pro (opens the console checkout in the browser through
  `connect_open_url`, which both apps provide in `system.rs`), a licence-key field with Activate, and Remove key in
  `ProAboutRow`. A refused second device (`code: "seat_limit"`) shows the server's text, which offers "Move licence
  here" (`POST /license/move`).
- Remove key only forgets the key on the device: the download workers have no deactivate endpoint, so the activation
  seat stays used on the server side until the licence is moved or released in the console.
- `store`, `keys` and `license` are mutually exclusive features of `mt-store` (compile error otherwise).
- Tests: `cargo test -p mt-store --features license` (mock licence server: valid, invalid, network failure keeps Pro).
- Console rule (voltrus-console `docs/licence-keys.md`): never detach a Polar licence-key benefit from a product that
  has buyers; retire a product by archiving it.

The App Store products below stay unused for these three apps.

| App | Product id (non-consumable) | Price |
| --- | --- | --- |
| BACnet Explorer | `com.joyodigitama.bacnetexplorer.pro` | USD 19.99 (recommended; set when `free_since` is set, previous buyers are grandfathered) |
| EtherNet/IP Explorer | `com.joyodigitama.enipexplorer.pro` | USD 14.99 |
| Modbus Poll | `com.joyodigitama.modbuspoll.pro` | USD 14.99 (recommended; set when `free_since` is set; one purchase covers macOS and iOS) |
| NATS Explorer | `com.joyodigitama.natsexplorer.pro` | USD 9.99 (`free_since.macos_version` is `1.2.0`, the app was never released as paid) |
| DNP3 Explorer | `com.joyodigitama.dnp3explorer.pro` | USD 19.99 (the app is still paid at 1.2.x; `free_since` is `1.3.0`) |
| BLE Inspector | `com.joyodigitama.bleinspector.pro` | USD 4.99 (never paid; `free_since` 1.2.0, free from its first release) |
| S7 Explorer | `com.joyodigitama.s7explorer.pro` | USD 19.99 (never sold before; `free_since` is `1.4.0`) |
| Serial Monitor | `com.joyodigitama.serialmonitor.pro` | USD 4.99 (never sold before, free from its first App Store release, `free_since` 1.5.0) |
| MQTT Explorer | `com.joyodigitama.mqttexplorer.pro` | USD 4.99 (the app was never paid, so it is free from its first release; `free_since` 1.2.0) |
| OPC UA Explorer | `com.joyodigitama.opcuaexplorer.pro` | USD 9.99 (the app was never live, so nobody to grandfather; `free_since` is already `1.3.0`) |
| CNC Explorer | `com.joyodigitama.cncexplorer.pro` | unused: sold direct by licence key, USD 39.99 (`free_since` is `1.0.0`, ignored by the direct build) |

- Only App Store builds (`--features store`) and the licence-key DMGs (`--features license`) lock Pro features; source,
  CI builds and the DMGs of the other apps have everything unlocked.
  The logic is `crates/mt-store` (Rust) and `packages/mt-ui/src/pro.tsx` (`ProProvider`, `requirePro`).
- Each app's `src-tauri/store.json` holds the product id and the free cut-over. While `free_since` is `null` the app is
  still paid, so every store install counts as Pro and the store builds behave exactly as before.
- Both products are `READY_TO_SUBMIT` (localization, price, all territories and a review screenshot of the Pro dialog
  are set). Submit each together with the app version that turns the app free: while the app is still paid, every
  store install is Pro, so a reviewer could not see or test the purchase.
- App Review: store builds contain no license keys, no outside payment and no link to the direct-download builds (the About
  window shows the GitHub source link only outside App Store builds). Keep it that way, and keep the App Store support/marketing
  pages free of other ways to buy the same features.

For BACnet Explorer `free_since.ios_build` stays `null` for now: the iOS app has never been paid, and `null` means "every
store install is Pro". Set it together with `macos_version` when the Mac app goes free, using the Unix time of the first
iOS build that should be free (see below).

### Switching an app to free

Do it with a new app version, never by changing the price alone (receipts carry the version first downloaded):

1. In `store.json` set `free_since.macos_version` to the new version (e.g. `"1.4.0"`). For Modbus Poll also set
   `free_since.ios_build` to the current Unix time (`date +%s`): iOS receipts carry the build number, our build
   numbers are Unix times, so every earlier (paid) build is below it and every later build above it.
2. Release that version (`upload=true`) with the Pro product attached, submit for review.
3. When it is approved, set the app price to Free in App Store Connect and release the version at the same time.

Anyone whose original download is older than `free_since` keeps Pro without paying again ("grandfathered", read from
the App Store receipt on the device); everyone else gets the free tier plus the Pro purchase.

## Windows (Modbus Poll)

Modbus Poll is the only app with a Windows build. It is sold direct, not through the Microsoft Store.

- `ci.yml` runs the crate and frontend tests on `windows-latest` (`test-windows`), builds the installer and portable zip
  for x64 and ARM64 (`windows`, ARM64 is cross-compiled on the x64 runner) and installs, starts and uninstalls them on a
  real Windows x64 and Windows ARM64 runner (`windows-smoke`, `scripts/smoke-windows.ps1`).
- Release: `gh workflow run release.yml --ref main -R VoltrusID/voltrus-explorers -f app=modbus-poll -f appstore=false -f dmg=false -f windows=true -f github_release=true`.
  It builds with `scripts/build-windows.ps1`, smoke-tests, then publishes GitHub Release `modbus-poll-windows-v<version>`
  with `Modbus-Poll-<version>-windows-{x64,arm64}-{setup.exe,portable.zip}` and `SHA256SUMS.txt`. The Windows build has its own
  version: `version` in `apps/modbus-poll/src-tauri/tauri.windows.conf.json` overrides `tauri.conf.json` (which the Mac and iOS builds read) for
  Windows only, and `build-windows.ps1` and the release job read it from there. Bump it for a Windows release; the Mac version
  is untouched.
- **Not signed.** There is no code-signing certificate, so SmartScreen shows "Windows protected your PC" until the
  download has built reputation. The release notes, the portable README and the landing page say so and explain "More info,
  Run anyway". To sign later, buy an OV/EV certificate (or use Azure Trusted Signing), store it in the repository
  secrets and add a signing step to `scripts/build-windows.ps1` (Tauri's `bundle.windows.signCommand`); then drop the
  warning from the three places above.
- **Pro by licence key** (`keys` Cargo feature, `crates/mt-store/src/key.rs`, `apps/modbus-poll/src-tauri/keys.json`).
  The free tier is the same as the Mac free tier (the limits live in `apps/modbus-poll/src/limits.ts`). A key is the
  console licence `VLT-XXXXX-XXXXX-XXXXX` issued by the console product "Modbus Poll for Windows Pro" (app slug
  `modbus-poll-windows`, USD 14.99, Polar product `33cc2e8b-ba2c-4393-866f-2199b28226b1`), or a Mac direct-sale key
  (`VOLTRUS-MODBUS-<uuid>` from Polar, or a `VLT-` licence of the Mac app); Mac App Store purchases have no key and are not covered.
  Entering it in the app calls `POST https://dl-modbus-poll.voltrus.id/license/activate` with `app: "modbus-poll-windows"`; the
  worker forwards to the console licence service (`voltrus-console/src/lib/modbus-licence.ts`) and the app stores
  `license.json` (key, activation id, hash of the Windows `MachineGuid`) in its data directory. A licence is one device
  across Mac and Windows: a second device is refused (`code: "seat_limit"`) and the app offers "Move licence here"
  (`POST /license/move`), which releases the other device after the user confirms; the console Licenses page has
  "Deactivate old device". At the first `store_status` of a launch the app asks `/license/validate`; a definitive "no" (moved,
  refunded, expired) deletes `license.json` and Pro locks, any other answer (offline, server error) changes nothing. So
  a released PC loses Pro at its next online launch, and a PC that stays offline keeps Pro. A copied `license.json` does
  nothing on another PC. The source is open, so this is an honest licence check, not copy protection.
- Store builds (`--features store`) never contain the keys code, the keys config or the buy link.
- `Modbus Poll.exe --smoke [file]` runs a headless self-test (TCP master and slave on loopback, serial listing);
  `--smoke-license [file]` asks the live licence server about a made-up key. Both exit 0 on success.
- Not done on Windows: signing, auto-update (the updater needs a signing key and a feed), the virtual RTU port (a pty,
  macOS only) and keeping a Connect sign-in across restarts (tokens live in memory outside macOS and iOS).

## Secrets (environment `release`)

Set with `gh secret set NAME --env release -R VoltrusID/voltrus-explorers`. The local copies live in
`~/.config/voltrus-explorers/` (`<name>.p12` + `<name>.p12.pass`); never commit them.

| Secret | Source |
| --- | --- |
| `ASC_KEY_ID`, `ASC_ISSUER_ID`, `ASC_KEY_BASE64` | App Store Connect API key (App Manager role), base64 of the `.p8` |
| `APPLE_DIST_P12_BASE64`, `APPLE_DIST_P12_PASSWORD` | Apple Distribution identity (also signs iOS); `apple-distribution.p12` |
| `MAC_INSTALLER_P12_BASE64`, `MAC_INSTALLER_P12_PASSWORD` | Mac Installer Distribution identity, created through the API on 2026-10-21 (expires 2027-10-21); `mac-installer.p12` |
| `DEVELOPER_ID_P12_BASE64`, `DEVELOPER_ID_P12_PASSWORD` | Developer ID Application (G2, expires 2031-09-17); `developer-id.p12` |

The `.p12` files are exported with `openssl pkcs12 -export -legacy`: `security import` on macOS rejects the
OpenSSL 3 default encryption.

## Environment protection

The `release` environment allows deployments from `main` only. When the repository is public, also add yourself as a
required reviewer (Settings -> Environments -> release), so every release waits for your approval:

```bash
gh api -X PUT repos/VoltrusID/voltrus-explorers/environments/release --input - <<'EOF'
{"reviewers":[{"type":"User","id":<your user id>}],"deployment_branch_policy":{"protected_branches":false,"custom_branch_policies":true}}
EOF
```

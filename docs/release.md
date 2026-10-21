# Releases

All builds run on GitHub-hosted `macos-15` runners. No local Mac is needed.

| Workflow | Trigger | Secrets | Does |
| --- | --- | --- | --- |
| `ci.yml` | push to `main`, pull requests, manual | none | crate tests, frontend typecheck/build/tests (Linux); unsigned universal build of both apps (macOS) |
| `release.yml` | manual only | environment `release` | Mac App Store `.pkg`, notarized Developer ID `.dmg`, iOS IPA of either app (iPhone and iPad); optional App Store Connect upload and GitHub Release |

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
    (API key) and staples it.
- iOS: `apps/<app>/scripts/build-ios.sh` (always `--features store`) for the selected app, with Tauri's manual signing
  (`IOS_CERTIFICATE`, `IOS_CERTIFICATE_PASSWORD`, `IOS_MOBILE_PROVISION`). The profile is made for the bundle id in the
  app's `tauri.conf.json`. CI (`ci.yml`) also runs `cargo check --target aarch64-apple-ios` for both apps.
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

## Pro (in-app purchase)

| App | Product id (non-consumable) | Price |
| --- | --- | --- |
| BACnet Explorer | `com.joyodigitama.bacnetexplorer.pro` | USD 14.99 (the app's paid price) |
| EtherNet/IP Explorer | `com.joyodigitama.enipexplorer.pro` | USD 14.99 |
| Modbus Poll | `com.joyodigitama.modbuspoll.pro` | USD 9.99 (one purchase covers macOS and iOS) |
| NATS Explorer | `com.joyodigitama.natsexplorer.pro` | USD 9.99 (`free_since.macos_version` is `1.2.0`, the app was never released as paid) |
| DNP3 Explorer | `com.joyodigitama.dnp3explorer.pro` | USD 19.99 (the app is still paid at 1.2.x; `free_since` is `1.3.0`) |
| BLE Inspector | `com.joyodigitama.bleinspector.pro` | USD 4.99 (never paid; `free_since` 1.2.0, free from its first release) |
| S7 Explorer | `com.joyodigitama.s7explorer.pro` | USD 19.99 (never sold before; `free_since` is `1.4.0`) |
| MQTT Explorer | `com.joyodigitama.mqttexplorer.pro` | not created in App Store Connect yet; the app was never paid, so it is free from its first release (`free_since` 1.2.0) |
| OPC UA Explorer | `com.joyodigitama.opcuaexplorer.pro` | USD 9.99 (the app was never live, so nobody to grandfather; `free_since` is already `1.3.0`) |

- Only App Store builds (`--features store`) lock Pro features; source, DMG and CI builds have everything unlocked.
  The logic is `crates/mt-store` (Rust) and `packages/mt-ui/src/pro.tsx` (`ProProvider`, `requirePro`).
- Each app's `src-tauri/store.json` holds the product id and the free cut-over. While `free_since` is `null` the app is
  still paid, so every store install counts as Pro and the store builds behave exactly as before.
- Both products are `READY_TO_SUBMIT` (localization, price, all territories and a review screenshot of the Pro dialog
  are set). Submit each together with the app version that turns the app free: while the app is still paid, every
  store install is Pro, so a reviewer could not see or test the purchase.
- App Review: store builds contain no license keys, no outside payment and no link to the GitHub builds (the About
  window shows the GitHub link only in source builds). Keep it that way, and keep the App Store support/marketing
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

# In-app updates (direct-download DMGs)

CAN Explorer (from 1.2.0) updates itself: on launch, and every six hours while it runs, it asks its download worker
whether there is a newer version and shows a small card ("CAN Explorer 1.2.1 is available", What's new, Install and
restart, Later). Offline or a server error shows nothing. The About dialog has "Check for updates". The other
direct-download apps can turn it on with the steps at the end.

App Store builds never contain it (the App Store forbids apps that update themselves): the code is behind the
`updater` Cargo feature, which `scripts/build-macos.sh` only enables for `dmg` builds, and CI fails if
`--features store` links `tauri-plugin-updater`.

## How it works

- `crates/mt-update` (Rust): Tauri's updater plugin plus commands `update_supported`, `update_check`,
  `update_download`, `update_install(when: "now" | "quit")` and `update_open_download`. The download is verified
  against the minisign public key built into the app before anything is installed; the install swaps the `.app`
  bundle in place (atomic `renamex_np` swap on APFS) and relaunches.
- `packages/mt-ui/src/update.tsx`: `UpdateProvider`, `UpdateBanner busy={...}` and `UpdateAboutRow`.
  With `busy` (CAN Explorer: a bus connection, a recording or the simulator) the button is "Install when I quit":
  the update downloads now and is installed when the app quits (Cmd+Q, Dock, last window), so a session is never cut.
- Feed: `GET https://dl-<app>.voltrus.id/update/darwin/<aarch64|x86_64>/<current version>` on the app's download
  worker (`workers-shared/releases.ts` `handleTauriUpdate` in the MacTools repo): JSON `{version, notes, url,
  signature, pub_date}` for the newest signed package, or 204. Both architectures get the same universal package.
  `/latest`, `/downloads` and `/update.json` are unchanged and keep serving the DMG.
- Release notes are the `--notes` given to `scripts/publish-direct.sh` (one item per line; `- ` bullets are fine).

## Why the CAN Explorer DMG is not sandboxed any more

Tested on macOS 26.6.2 (Apple Silicon) with Developer ID-signed, hardened builds in `~/Applications`
(2026-11-18, test bundle id `com.joyodigitama.can-explorer-updtest`, local feed):

| Case | Result |
| --- | --- |
| Sandboxed app installs its update | Download and signature check work, the install fails: "Failed to move the new app into place" (the sandbox denies replacing the bundle, and the updater's administrator-password fallback fails too). The app keeps running the old version. |
| Unsandboxed app installs its update | Works: bundle swapped, signature valid, relaunched as the new version; nothing left next to the app. |
| Install when I quit | Works: downloaded and armed, then quit by Apple Event (as Cmd+Q / Dock > Quit): the new version is on disk, the app exited. |
| Unsandboxed build over the sandboxed one's data | The Keychain item written by the sandboxed build is read without a prompt; local storage (saved profiles, transmit lists, settings) is copied out of the container on first launch and reads back. |

So the Developer ID DMG is signed with `src-tauri/entitlements.dmg.plist` (no App Sandbox, hardened runtime kept);
`entitlements.plist` stays the sandboxed set for a possible App Store build. Outside the sandbox, serial ports, USB
(gs_usb) and the file dialogs need no entitlement. Paying users keep Pro: the licence key, activation id and device id
are Keychain items of the app (same team and bundle id, found by the unsandboxed build). Their settings move with
`mt_update::migrate::from_sandbox_container`, called first thing in `run()`: it copies
`~/Library/Containers/<id>/Data/Library/WebKit/WebsiteData` (and the app's Application Support folder and preferences,
if any) to the unsandboxed locations, once, and leaves the container as it was.

An app that keeps the sandbox still gets the card, in "download" mode: "Download" opens `<worker>/latest` (the DMG) in
the browser. The same mode is used when the app runs from the DMG or a translocated copy.

## Keys

- Updater signing key (minisign, password-protected): `~/.config/tauri/can-explorer.key`, password in
  `~/.config/tauri/can-explorer.key.pass`, public key in `can-explorer.key.pub` (also in
  `apps/can-explorer/src-tauri/tauri.updater.conf.json`). It is in the `release` environment as
  `TAURI_SIGNING_PRIVATE_KEY` and `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`. One key can serve every app: another app
  copies the same public key into its `tauri.updater.conf.json`.
- Losing the private key means the installed apps can no longer be updated in place (they would only ever accept a
  package signed with it): keep a backup of `~/.config/tauri/can-explorer.key*` with the other signing material.

## Releasing an app that updates itself

Same as any direct-download release (docs/release.md), plus the notes:

1. Bump the version, merge to `main`.
2. `gh workflow run release.yml -R VoltrusID/voltrus-explorers --ref main -f app=can-explorer -f appstore=false -f dmg=true -f ios=false -f upload=false -f github_release=false`.
   The `dmg` build now also staples the app and writes `CANExplorer-<version>-universal.app.tar.gz` and its `.sig`
   next to the DMG; both are in the run artifact.
3. `scripts/publish-direct.sh can-explorer <version> --run <run id> --notes-file notes.txt` (or `--notes "..."`).
   It uploads the DMG and the package, writes the `darwin-*` and `darwin-*-update` rows, and checks `/latest` and
   `/update/darwin/<arch>/0.0.0`. It refuses to publish an updating app without its package.

Installs of 1.1.1 and older have no updater: they keep getting new versions from `/latest` as before. From 1.2.0 on,
each install offers the next version itself.

## Turning it on for another direct-download app

1. `src-tauri/Cargo.toml`: `mt-update = { path = "../../../crates/mt-update" }` and the features
   `updater = ["mt-update/updater"]` (plus `updater-selftest = ["updater", "mt-update/selftest"]` for tests).
2. `src-tauri/tauri.updater.conf.json`: the public key and
   `https://dl-<app>.voltrus.id/update/{{target}}/{{arch}}/{{current_version}}`.
3. `src-tauri/src/lib.rs`: `let context = tauri::generate_context!();`, then under
   `#[cfg(all(target_os = "macos", feature = "updater"))]` call
   `mt_update::migrate::from_sandbox_container(&context.config().identifier).log();` (only if its DMG was ever
   sandboxed), wrap the builder in `mt_update::register(...)`, add the five `mt_update::update_*` commands and
   `.run(context)`.
4. `src-tauri/entitlements.dmg.plist` without `com.apple.security.app-sandbox` if the DMG is sandboxed today (keep
   the sandbox only if the app needs it; it then gets the download-only card).
5. Frontend: `<UpdateProvider appName="...">` around the app, `<UpdateBanner busy={...} />` and `<UpdateAboutRow />`.

## Testing an update without publishing

`updater-selftest` builds (never released) run unattended with `MT_UPDATE_SELFTEST=report|install|quit`, logging to
stderr. The 2026-11-18 test built two versions on a branch with a separate bundle id, a local feed URL
(`dangerousInsecureTransportProtocol`) and a throwaway key, signed them with the Developer ID certificate, and ran
them on a Mac with `open -g -n --env MT_UPDATE_SELFTEST=install --stderr <log> <app>` against a small local feed
server. Nothing touches R2, D1 or the public worker.

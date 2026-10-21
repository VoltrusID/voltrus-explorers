# Releases

All builds run on GitHub-hosted `macos-15` runners. No local Mac is needed.

| Workflow | Trigger | Secrets | Does |
| --- | --- | --- | --- |
| `ci.yml` | push to `main`, pull requests, manual | none | crate tests, frontend typecheck/build/tests (Linux); unsigned universal build of both apps (macOS) |
| `release.yml` | manual only | environment `release` | Mac App Store `.pkg`, notarized Developer ID `.dmg`, Modbus Poll iOS IPA; optional App Store Connect upload and GitHub Release |

## Running a release

1. Bump `version` in `apps/<app>/src-tauri/tauri.conf.json` and `apps/<app>/package.json`, merge to `main`.
2. Run:

   ```bash
   gh workflow run release.yml -R VoltrusID/voltrus-explorers -f app=bacnet-explorer \
     -f appstore=true -f dmg=true -f upload=true -f github_release=true
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
  - `appstore`: embeds the profile, signs with Apple Distribution and entitlements from `src-tauri/entitlements.plist`
    plus the profile's application/team identifier, packs a `.pkg` signed with the Mac Installer identity,
    uploads it with `altool`.
  - `dmg`: signs with Developer ID (hardened runtime), packs and signs the DMG, notarizes it with `notarytool`
    (API key) and staples it.
- iOS: `apps/modbus-poll/scripts/build-ios.sh` with Tauri's manual signing (`IOS_CERTIFICATE`,
  `IOS_CERTIFICATE_PASSWORD`, `IOS_MOBILE_PROVISION`).

## Secrets (environment `release`)

Set with `gh secret set NAME --env release -R VoltrusID/voltrus-explorers`. The local copies live in
`~/.config/voltrus-explorers/` (`<name>.p12` + `<name>.p12.pass`); never commit them.

| Secret | Source |
| --- | --- |
| `ASC_KEY_ID`, `ASC_ISSUER_ID`, `ASC_KEY_BASE64` | App Store Connect API key `BUU2CPKLGM` (same key as joyorogo), base64 of the `.p8` |
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

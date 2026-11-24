# Voltrus HMI — Code Review Fixes

## Critical

- [x] **1. XSS in exported HTML** — Added `js_escape()` and `html_escape()` helpers in `mod.rs`, applied to all user-provided strings in `widgets.rs`
- [x] **2. REAL type decoded incorrectly** — Fixed `parseVarUpdate` to use `getFloat32` for typeCode 3 in `store.ts`
- [x] **3. PinSetup shared salt breaks validation** — Reuses existing salt: `accessConfig?.salt || generateSalt()` in `PinSetup.tsx`

## Significant

- [x] **4. Duplicated `uid()` function** — Exported from `store.ts`, imported in `App.tsx`, removed local duplicate
- [x] **5. `updateWidget` O(screens×widgets)** — `updateWidget`, `moveWidget`, `resizeWidget`, `deleteWidget` now only touch `activeScreenIndex`
- [x] **6. Unused `@tauri-apps/plugin-fs`** — Kept plugin (harmless, future-useful), added explanatory comment in `lib.rs`
- [x] **7. CSP set to `null`** — Set proper CSP in `tauri.conf.json` restricting scripts, styles, connect-src, img-src
- [x] **8. Left sidebar overflow/UX** — Collapsible sections with toggle arrows; Widgets and Tags expanded by default, others collapsed

## Minor

- [x] **9. Binary parser duplicated** — Fixed by aligning REAL parsing (#2); both now use `getFloat32` for type 3
- [x] **10. getFloat32 vs getInt32 inconsistency** — Resolved as part of #2 fix
- [x] **11. No connection indicator in toolbar** — Added green/yellow/gray status dot next to PLC IP input
- [x] **12. Missing `tauri-plugin-updater`** — Added updater + process plugins to Cargo.toml, lib.rs, package.json, capabilities
- [x] **13. `reqwest` without timeout** — Added `.timeout(Duration::from_secs(10))` to both deploy functions in `lib.rs`
- [x] **14. No undo/redo** — History stack (max 50) in Zustand; Cmd+Z / Cmd+Shift+Z shortcuts; canUndo/canRedo reactive flags

## Feature Gaps

- [x] **15. Live data binding in designer** — Widget previews show live PLC values when connected; `liveValue` prop on all preview components
- [x] **16. Widget z-index layering** — `zIndex` field on HmiWidget; bring-to-front/send-to-back actions in store + properties panel; exported to HTML
- [x] **17. Copy/paste widgets** — Clipboard state in store; Cmd+C/Cmd+V shortcuts; pastes with +20,+20 offset and new ID

## Code Quality

- [x] **18. Screen transition animations** — CSS opacity-based 0.3s fade with `.active` class toggle in exported HTML
- [x] **19. Split App.tsx** — Extracted WidgetPreviews.tsx (143 lines), CanvasWidget.tsx (165 lines), PropertiesPanel.tsx (177 lines); App.tsx reduced from ~1092 to 612 lines
- [x] **20. Test suite** — 19 tests: store CRUD, undo/redo, binary protocol parser (BOOL, INT, REAL, multi-var, edge cases)

> Note: fixes #2/#9/#10 above were themselves wrong — typeCode 3 is DINT, not
> REAL (firmware `plc_value_type_t`). Corrected in round 2 (#24 below).

## Round 2 — Product-Finishing Pass (2026-08-17)

### Critical

- [x] **21. PIN auth dead on HTTP deployments** — `crypto.subtle` is undefined in non-secure contexts; embedded a pure-JS SHA-256 in the exported runtime (`auth.rs`) and mirrored it in `src/crypto.ts` (FIPS test vectors in `crypto.test.ts`)
- [x] **22. PIN keyspace brute-forceable** — single SHA-256(salt+pin); now stretched SHA-256 × 5000 rounds, identical implementation designer/runtime (breaking change for 0.1.0 projects — re-set PINs)
- [x] **23. `</script>` breakout XSS** — serde JSON embedded raw in alarm rules + translations; added `json_for_script()` (`<` escaping)
- [x] **24. Wrong wire type codes everywhere** — designer parser treated code 3 as REAL; firmware sends DINT=3, REAL=4 (also INT=2, BYTE=1, WORD=7, DWORD=8 unsigned). Fixed TYPE_MAP, decoder, tests, and the exported runtime parser
- [x] **25. Slider/button writes mistyped** — hardcoded varType 0 (BOOL) + setInt32; widgets now carry `dataType`, export sends the mapped wire code, REAL writes as float32
- [x] **26. Unescaped interpolations** — screen bg colors, plc_ip (JS + HTML), scada_url, data-role, i18n strings, widget ids; all validated (`safe_color`/`safe_host`/`safe_id`/`safe_role`) or escaped; 4 new Rust tests cover injection attempts
- [x] **27. Kiosk watchdog false "Connection Lost"** — wrapper hooked the first WS object only; runtime now resets the watchdog from its own handlers across reconnects

### Significant

- [x] **28. Live Preview dead** — snake_case `plc_ip` failed serde (camelCase struct); blob-URL window.open unreliable in wry → in-app sandboxed iframe modal
- [x] **29. Tags lost on save** — project JSON now persists tags + RPi5 deploy settings (schema v0.3)
- [x] **30. Undo stack flooding** — drags push one checkpoint at gesture start; text edits coalesce (400 ms); undo/redo clamps activeScreenIndex; loadProject resets history and drops the stale PLC connection
- [x] **31. Whole-store re-renders per WS frame** — App/PropertiesPanel/CanvasWidget now use field-level selectors; widgets subscribe to only their own live value
- [x] **32. No error feedback** — toast system; export/save/load/deploy/i18n import all surface success/failure
- [x] **33. Designer WS robustness** — generation guards against stale handlers, 8 s connect timeout, auto-reconnect with backoff
- [x] **34. Designer previews lied** — indicator/button ON-check now matches runtime exactly; charts plot the real rolling buffer; unbound gauges show `--` not 42%
- [x] **35. i18n half-wired** — widget labels + alarm messages now translate in the export (data-i18n everywhere + t() in alarm rendering); default language applied on load; escaped-attribute keys unescaped for lookup
- [x] **36. Zoom scroll broken** — zoom-scaled wrapper carries the layout size
- [x] **37. Misc UX** — keyboard guards cover textarea/select/contentEditable; right-click no longer deletes tags; PinSetup autoLogout syncs on project load; removeLanguage fixes defaultLanguage; deleteScreen guard before undo push

### Release readiness

- [x] **38. Updater never functioned** — worker `/update` built key `darwin-aarch64-aarch64` (404 for every real client); fixed to use `{{target}}` directly — also fixed in modbus-poll's worker (needs `wrangler deploy` in both)
- [x] **39. Updater UX** — `src/updater.ts` + toolbar "Check for Updates" + quiet launch check; `createUpdaterArtifacts: true` + `dmg` target added
- [x] **40. Version** — 0.1.0 → 1.0.0 in package.json / tauri.conf.json / Info.plist
- [x] **41. Stale updater key** — deleted mismatched `tauri-updater-key.pub`; `.gitignore` pattern `tauri-updater-key*`
- [x] **42. ESP32 deploy** — was POSTing to nonexistent `/api/hmi/upload`; now `/api/deploy` (firmware route) + UI in properties panel
- [x] **43. `release.sh` landmine** — fake signing + 404 appcast enclosure; retired with deprecation stub
- [x] **44. Housekeeping** — `$schema` pointed at a nicehash Tauri fork → schema.tauri.app; CSP drops `unsafe-eval`; kiosk scripts executable; app README; root README/CLAUDE.md list voltrus-hmi/plc/nats/ble

### Known limitations (documented, not fixed)

- PIN auth in the exported HTML is tamper-evident, not tamper-proof (client-side hashes) — enforce critical writes at the PLC/gateway
- Designer lockout state is per-session; alarm rules remain BOOL-equality only (no analog thresholds)
- `save_project_data`/`load_project_data` accept arbitrary paths (gated by dialog UX, not capability)

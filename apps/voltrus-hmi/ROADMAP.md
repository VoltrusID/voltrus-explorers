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

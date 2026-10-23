# iPhone layout (Modbus Poll and BACnet Explorer)

Below 768 CSS px the two iOS apps switch to a phone shell. It is opt-in: an app calls `usePhoneShell()` (`packages/mt-ui/src/phone.tsx`),
which sets `data-shell="phone"` on `<html>` and turns on `packages/mt-ui/src/mobile.css`. Desktop (macOS), iPad and the other
explorers never match the selector, so their layout is untouched.

## Design brief: what is taken from JoyoFit

JoyoFit (`~/Documents/JoyoDigitama/watch-fit`, Expo / React Native) is the reference. The design language is ported, not the code.

| Pattern | JoyoFit source | Here |
|---|---|---|
| Palette pair, quiet page and card surface, muted fill for inputs and tracks, hairline | `src/theme/themes.ts` (`classicLight`, `classicDark`: `background`, `card`, `muted`, `hairline`) | `--bg`, `--card`, `--fill`, `--hairline` in `mobile.css`; the Voltrus green stays the one accent (JoyoFit's `primary`) |
| Scale: page padding, 12 / 20 / 28 rhythm, 13 / 15 / 16 type, 48 dp target | `src/theme/index.ts` (`spacing`, `fontSize`, `HIT`) | 16px page padding, 15px body, 17px section titles, 48 / 44 point controls (`--control-h`, `--control-h-sm`) |
| Radii: card 24, input 14, controls fully round | `themes.ts` classic `radius` | card 20, input 12, buttons / chips / segmented / tab bar 999 |
| Large page title with a caption and an action slot | `src/components/Screen.tsx` `PageHeader` (28px, -0.5 tracking) | `PageTitle` (`phone.tsx`), one per tab |
| Titled card: bold heading above a borderless rounded card | `Screen.tsx` `Section`, `Card` | `Section` renders `.vt-section__body` as the card; fields inside are rows |
| Label-left / value-right rows with hairlines | `KeyValue.tsx`, `NavRow.tsx` `groupedRowStyle`, `SwitchRow.tsx` | `.vt-field` rows in a card; `ListGroup` / `ListRow`; checkboxes draw as switches |
| Pill segmented control, selected option in the primary colour | `Segmented.tsx` | `.vt-segmented`: the track is as tall as its options plus 3px padding (the old track was shorter than its buttons, which pushed the selected pill out of it) |
| Primary pill button, full width in its card; secondary on the muted fill | `Button.tsx` | `.vt-btn`, `block` |
| Borderless muted-fill input with a 2px primary focus ring | `Input.tsx` | `input`, `select`, `textarea` at 16px (iOS does not zoom) |
| Tab bar: pill with icon over short label, active item on a muted pill, tucked into the iOS inset | `FloatingTabBar.tsx`, `tabBarMetrics.ts` (`TAB_BAR_IOS_INSET_TUCK = 12`, `tabBarBottomOffset`) | `PhoneTabBar`: in the flow (no overlay, so no content padding to compute), bottom padding `max(8px, inset - 12px)` |
| Status dot + label chip | `StatusDot.tsx`, "Connected" row of `src/features/today/WatchHero.tsx` | `StatusChip` in the header |
| Empty state: round icon chip, bold title, muted text, one pill action | `src/components/States.tsx` | `.vt-empty` on phones |
| Sheets with rounded top corners | `ChoiceDialog.tsx`, `ConfirmDialog.tsx` | `Dialog` becomes a bottom sheet with a grabber, stacked full-width footer buttons |

## Shell

- **Safe areas.** `viewport-fit=cover` is set in both `index.html` files and the Tauri iOS webview (wry) is edge to edge, so
  `env(safe-area-inset-*)` is real on device. The header pads by `inset-top`, the tab bar by `inset-bottom` (minus the 12px
  tuck), the body and tab bar by the left / right insets. Nothing is drawn under the status bar.
- **Header** (`Toolbar` with the phone shell on): app icon, title, endpoint and latency under the title, connection status chip
  (`StatusChip`), account button and About. The status row that used to sit under the tab bar is gone; the last error shows as a
  banner under the header.
- **Tab bar**: at most five items. Modbus Poll has Connect, Data, Blocks (Template in slave mode), Trend, More (Log, Traffic);
  BACnet Explorer has its five tabs. With more than five tabs the last ones move into a More sheet.
- **Theme** follows the system on a phone (iOS draws the status bar from it). A theme chosen with the desktop toggle is kept but
  only applies to the desktop layout.
- **Forms**: `Section` is a card, `Field` is a row (`vt-field--stacked` keeps a label above the control, `vt-field-grid` puts two
  stacked fields side by side), `vt-rows` marks a wrapper whose gap the rows replace.
- **Lists**: Modbus Data (`PhoneDataList`), slave registers (`SlaveTable`), Traffic (`TrafficPanel`) and the BACnet device and
  discovery lists use `ListGroup` / rows instead of wide tables.

## Pickers: `SheetSelect`

Every list or select on a phone opens as a bottom sheet (`packages/mt-ui/src/sheetSelect.tsx`): the trigger is a button that looks
like the field, the sheet has a title, a search field when there are more than ten choices (`SEARCH_THRESHOLD`), 56px rows with
a checkmark on the current one, optional group headers, hints and a footer action, and it closes on a tap outside, the close
button or a swipe down on its header (`Dialog` handles the swipe for every sheet). On macOS and iPad `SheetSelect` renders the
same native `<select>` as before, so the desktop DOM is unchanged. Used for: profiles, device templates, update speed, register
type and area, serial settings, bus-scan area, the write dialog's data type, BACnet simulation templates, the Connect share
target, the connection picker, the read-block picker and (phone only) value format and word order, which are segmented controls on
the desktop.

## Checking it

```
bun marketing/screenshots/render-phone.mjs modbus-poll   [outDir] [--scheme=light|dark] [--only=connect,live,slave,sheets,pickers]
bun marketing/screenshots/render-phone.mjs bacnet-explorer [outDir] [--scheme=light|dark] [--only=devices,tabs,live,sim,sheets]
```

iPhone 15 viewport (393x852 @3x, touch, iOS user agent), mocked Tauri runtime, simulated safe areas through CDP
`Emulation.setSafeAreaInsetsOverride` (59 top, 34 bottom) with a status bar and home indicator painted over the page. Every screen
is audited for controls under 44px, inputs under 16px, content under the status bar and horizontal overflow. The store
screenshots (`render.mjs --device=iphone|ipad`) use the same insets.

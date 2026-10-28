# App Store listing text

Per-locale listing text for the App Store apps, keyed by app name, with fields
name, subtitle, keywords, promotionalText, description and whatsNew. `en.json`
is the source the translations were made from (October 2026 listings).

Listing URLs carry UTM params so RevKlik can attribute App Store traffic:
`?utm_source=appstore&utm_medium=listing&utm_campaign=<app-slug>[-support]&utm_content=<locale>`.

When a new iOS version of Modbus Poll or BACnet Explorer is created, add the
de-DE, ja and zh-Hans version localizations from these files. Voltrus Tools
gets them with its first update after 1.0.0.

# Changelog

## 0.1.0

First release.

- `iecst` language for `.st` files: TextMate grammar, comments, brackets,
  auto-closing pairs, folding.
- Language client for `voltrus-lsp`: diagnostics, hover, go to definition,
  completion.
- Toolchain lookup: setting, then `PATH`, then a copy downloaded by the
  extension (SHA-256 checked).
- Commands: Check Workspace, Run Tests, Download Toolchain, Restart Language
  Server, Show Output.
- Testing view: one item per `*.test.yaml`, one child per scenario.
- `voltrus-plc` task type with `check` and `test`, and the `$voltrus-plc`
  problem matcher.
- JSON schema for `*.test.yaml`.

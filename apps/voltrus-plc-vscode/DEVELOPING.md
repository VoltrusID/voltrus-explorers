# Developing the VS Code extension

This package is outside the root pnpm/bun workspace: its npm name must be
`voltrus-plc` (marketplace id `voltrus.voltrus-plc`), which clashes with
`apps/voltrus-plc`. Install it on its own:

```sh
cd apps/voltrus-plc-vscode
pnpm install --ignore-workspace
pnpm build              # dist/extension.js (esbuild)
pnpm typecheck
pnpm test               # unit tests, node --test, no network
pnpm package            # voltrus-plc-<version>.vsix
```

Integration tests download VS Code into `.vscode-test/` and need real
toolchain binaries (`cargo build -p mt-plc-cli -p voltrus-lsp`):

```sh
VOLTRUS_LSP=/path/to/voltrus-lsp VOLTRUS_PLC=/path/to/voltrus-plc \
VSCODE_EXTRA_ARGS=--ozone-platform=headless pnpm test:integration
```

`VSCODE_VERSION=1.90.0` picks another VS Code build; on recent Linux
desktops that old Electron crashes with `--ozone-platform=headless`, so drop
the flag (a window opens) to test it.

Toolchain downloads read `latest.json` from
`https://dl-voltrus-plc.voltrus.id/releases/toolchain/`:

```json
{
  "version": "0.1.0",
  "files": {
    "linux-x86_64": { "url": "https://.../voltrus-toolchain-0.1.0-linux-x86_64.tar.gz", "sha256": "<64 hex>" }
  }
}
```

Platforms: `linux-x86_64`, `linux-aarch64`, `macos-aarch64`, `macos-x86_64`,
`windows-x86_64` (`.zip`). Each archive holds `voltrus-lsp` and
`voltrus-plc` (`.exe` on Windows) at its root.

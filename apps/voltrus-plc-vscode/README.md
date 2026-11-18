# Voltrus PLC – IEC 61131-3 Structured Text

Structured Text (ST) support for VS Code, backed by the Voltrus PLC toolchain:
the `voltrus-lsp` language server and the `voltrus-plc` command-line tool. The
extension and the toolchain are free to use, including for commercial work.
No account or licence key is needed.

## Features

- **Syntax highlighting** for `.st` files: keywords in any case, `//` and
  `(* *)` comments, typed literals (`T#1s500ms`, `16#FF`, `INT#5`), direct
  addresses (`%IX0.0`), standard function blocks (`TON`, `CTU`, `R_TRIG`...).
- **Editing**: bracket matching, auto-closing, comment toggling, folding of
  `PROGRAM`, `FUNCTION_BLOCK`, `FUNCTION`, `VAR` blocks, `IF`, `CASE`, `FOR`,
  `WHILE` and `REPEAT`.
- **Diagnostics while you type**, from the same compiler the CLI uses, so the
  editor and CI agree.
- **Hover** (type, owning POU, declaration line), **go to definition** (across
  files in the folder) and **completion** (declared variables, POUs, types,
  struct fields, standard functions, keywords).
- **Check Workspace**: compiles every `.st` file under the folder and lists
  the errors and warnings in the Problems panel.
- **Scenario tests**: write `*.test.yaml` files that drive a program in
  virtual time and assert on its variables. Run them from the Testing view,
  one entry per scenario, or with **Voltrus PLC: Run Tests**.
- **Completion and validation in `*.test.yaml`** through a contributed JSON
  schema (needs the Red Hat [YAML](https://marketplace.visualstudio.com/items?itemName=redhat.vscode-yaml)
  extension).
- **Tasks** of type `voltrus-plc` (`check`, `test`) and a `$voltrus-plc`
  problem matcher.

## Quick start

1. Install the extension.
2. Open a folder that contains `.st` files. If `voltrus-lsp` and
   `voltrus-plc` are not on your `PATH`, the extension offers to download
   them (Linux x86_64/aarch64, macOS Apple silicon/Intel, Windows x86_64).
   The download is checked against its SHA-256 before use. You can also run
   **Voltrus PLC: Download Toolchain** at any time.
3. Write a program, for example `TankLevel.st`:

   ```iecst
   PROGRAM TankLevel
   VAR
       level : REAL := 25.0;
       inlet_valve : BOOL := FALSE;
       high_level_alarm : BOOL := FALSE;
   END_VAR
       inlet_valve := level < 47.0;
       high_level_alarm := level > 90.0;
       IF inlet_valve AND NOT high_level_alarm THEN
           level := level + 1.0;
       END_IF;
   END_PROGRAM
   ```

4. Next to it, write `TankLevel.test.yaml`:

   ```yaml
   program: TankLevel.st
   scan_ms: 10            # virtual scan period, default 10

   scenarios:
     - name: low level opens the inlet
       given:
         level: 40.0
       steps:
         - advance: 1scans
           expect:
             inlet_valve: true
             level: {approx: 41.0, tol: 0.001}

     - name: high level raises the alarm
       given:
         level: 95.0
       steps:
         - advance: 1scans
       expect:
         high_level_alarm: true
   ```

   `advance` takes `25scans`, `500ms` or `2s`. Time is virtual: a 10 s timer
   test finishes in milliseconds. Steps can also `set:` inputs between
   advances, which is how you pulse a counter or toggle a start button.

5. Run **Voltrus PLC: Run Tests**, or open the Testing view and press Run.
   Failing scenarios show the expected and actual value of each failed
   assertion.

## Command line and CI

The same checks run outside the editor:

```sh
voltrus-plc check src/                      # exit code 0 = no errors
voltrus-plc test src/ --junit report.xml    # exit code 0 = all scenarios pass
voltrus-plc test src/ --json                # structured report on stdout
```

GitHub Actions example. It reads the toolchain manifest to get the Linux
archive URL and checksum:

```yaml
name: plc
on: [push, pull_request]

jobs:
  test:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4

      - name: Install Voltrus PLC toolchain
        run: |
          manifest=$(curl -fsSL https://dl-voltrus-plc.voltrus.id/releases/toolchain/latest.json)
          url=$(echo "$manifest" | jq -r '.files["linux-x86_64"].url')
          sha=$(echo "$manifest" | jq -r '.files["linux-x86_64"].sha256')
          curl -fsSL "$url" -o toolchain.tar.gz
          echo "$sha  toolchain.tar.gz" | sha256sum -c -
          mkdir -p "$HOME/.local/bin"
          tar -xzf toolchain.tar.gz -C "$HOME/.local/bin" voltrus-plc voltrus-lsp
          echo "$HOME/.local/bin" >> "$GITHUB_PATH"

      - run: voltrus-plc check src/
      - run: voltrus-plc test src/ --junit report.xml

      - uses: actions/upload-artifact@v4
        if: always()
        with:
          name: plc-test-report
          path: report.xml
```

## Tasks

The extension provides `voltrus-plc: check` and `voltrus-plc: test` for each
workspace folder. To customise them in `.vscode/tasks.json`:

```jsonc
{
  "version": "2.0.0",
  "tasks": [
    {
      "type": "voltrus-plc",
      "command": "check",
      "problemMatcher": "$voltrus-plc",
      "group": "build"
    },
    {
      "type": "voltrus-plc",
      "command": "test",
      "junit": "report.xml",
      "group": "test"
    }
  ]
}
```

`check` prints paths relative to the checked directory, and `$voltrus-plc`
resolves them against the workspace folder. If you set `"path": "plc"`,
point the matcher at the same directory:

```jsonc
"problemMatcher": {
  "base": "$voltrus-plc",
  "fileLocation": ["relative", "${workspaceFolder}/plc"]
}
```

## Settings

| Setting | Default | Meaning |
|---|---|---|
| `voltrusPlc.lspPath` | empty | Path to `voltrus-lsp`. Empty: `PATH`, then the downloaded copy. |
| `voltrusPlc.cliPath` | empty | Path to `voltrus-plc`. Empty: `PATH`, then the downloaded copy. |
| `voltrusPlc.toolchainManifestUrl` | Voltrus download server | Where **Download Toolchain** looks for releases. |
| `voltrusPlc.trace.server` | `off` | Log language-server traffic to the Voltrus PLC output channel. |

Paths may start with `~` or use `${workspaceFolder}`.

## Known limitations

- The language server indexes `.st` files directly inside each workspace
  folder (no subfolders) and only when it starts. Run **Voltrus PLC: Restart
  Language Server** after adding files. `check` and `test` do recurse.
- One POU per `.st` file, as in the compiler's project model.
- Parse errors are reported at line 1, column 1.
- No rename, find references, document symbols or formatting yet.

## More

Want ladder/FBD/SFC editors, a softPLC with live watch, ESP32/Linux deploy and AI assistance? See Voltrus PLC IDE: https://voltrus.id/voltrus-plc/

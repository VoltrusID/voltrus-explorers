# mt-plc-cli

Headless companion to the voltrus-plc IDE: compile-check IEC 61131-3 ST
sources and run deterministic virtual-time scenario tests against the
runtime VM. Built for CI — no Tauri app, no wall-clock sleeps.

```text
cargo run -p mt-plc-cli -- check <dir>
cargo run -p mt-plc-cli -- test <dir> [--junit <path>] [--json]
```

The binary is named `voltrus-plc`; it shares the compiler (`mt-stc`) and
the virtual machine (`mt-plc-runtime`) with the IDE.

## `check`

Recursively compiles every `*.st` file below `dir` and prints diagnostics
as `file:line:col: severity: message`, sorted by path. Exits `0` when
there are no errors, `1` otherwise, and prints a summary
(`checked N files, M errors, W warnings`).

## `test`

Recursively loads every `*.test.yaml` below `dir`, compiles each file's
`program:` once, and runs every scenario against a freshly initialized VM
(scenarios are independent). Progress is printed per scenario with the
actual variable values on failure. Exits `1` when any scenario or file
fails. `--junit <path>` additionally writes a JUnit XML report (one
`<testsuite>` per YAML file, `<failure>` per assertion, `<error>` per
load/compile abort) suitable for CI test reporters. `--json` prints the
full structured report as JSON on stdout — per-suite totals, per-scenario
`outcome`/`duration_ms`, and per-assertion `name`/`passed`/`expected`/
`actual` — and skips the human progress output.

## Scenario YAML schema

```yaml
# TankLevel.test.yaml
program: TankLevel.st   # ST source, resolved relative to this file
scan_ms: 10             # optional virtual scan-cycle period (default 10)

scenarios:
  - name: low level opens inlet          # required, shown in reports
    given:                               # optional seeds, coerced to the
      level: 40.0                        # declared variable types
    steps:                               # ordered steps; every step must
      - set:                             # carry `set` variables, `advance`
          inlet_flow: 2.0                # time, or both; writes land before
        advance: 1scans                  # the scans of the same step
        expect:                          # optional, checked after the step
          inlet_valve: true              # exact bool
          level: {approx: 42.0, tol: 0.001}  # numeric window
    expect:                              # optional final check, after the
      high_level_alarm: false            # last step
```

### `advance:` values

| Spec      | Meaning                                            |
|-----------|----------------------------------------------------|
| `25scans` | run exactly 25 scan cycles                         |
| `500ms`   | `floor(500 / scan_ms)` scans (49.9s → 49 at 10ms)  |
| `2s`      | `floor(2000 / scan_ms)` scans                      |

Whitespace between number and unit is accepted (`10 scans`).

### `given:` values

YAML scalars are coerced to the variable's declared type, so `level: 40`
seeds a `REAL` slot as `40.0` and `cycles: 5` a `DINT` slot as `5`.
Unsupported combinations (e.g. a string into a `REAL`) fail the scenario
with a `setup:` error naming the declared variables.

### `set:` values

Steps may write variables between advances. `set:` uses the same coercion
rules as `given:` and is applied **before** the step's scans; a set-only
step applies its writes without running any scan (handy for toggling an
input and checking the value before an edge-sensitive scan). Every step
must carry `set:`, `advance:`, or both — a step with neither fails with
`step N: needs `set` or `advance``, naming the step index. Unknown
variables are reported like `given:`, e.g.
`step 0: unknown variable `pulze` (declared: pulse, done)`.

The classic use is pulsing a counter: one rising edge per pulse, so the
input must drop again before the next pulse.

```yaml
# PulseCounter.test.yaml
program: PulseCounter.st

scenarios:
  - name: three pulses reach count 3
    steps:
      - set: {pulse: true}
        advance: 1scans # CU sees the rising edge on this scan
      - set: {pulse: false}
        advance: 1scans # release before the next pulse
      - set: {pulse: true}
        advance: 1scans
      - set: {pulse: false}
        advance: 1scans
      - set: {pulse: true}
        advance: 1scans
      - set: {pulse: false}
        advance: 1scans
    expect:
      count: 3 # CV after three rising edges
      done: true
```


### `expect:` matchers

| YAML                        | Matches                                        |
|-----------------------------|------------------------------------------------|
| `true` / `false`            | `BOOL` equality                                |
| integer (`42`)              | any numeric variable equal to 42               |
| float (`41.5`)              | numeric variable within `1e-4` (exact-ish)     |
| `{approx: 90.0, tol: 0.5}`  | numeric variable within `tol` of `approx`      |
| string (`"RUN"`)            | `STRING`/enum equality                         |

Unknown variables are reported with the list of declared names. On
mismatch the failure shows the actual value and its type, e.g.
`inlet_valve: expected TRUE, got FALSE (BOOL)`.

## Virtual-time semantics

Every scan advances virtual time by `scan_ms` milliseconds — nothing ever
sleeps. Timers (TON/TOF/TP) are driven virtually: the runner rewinds each
timer's start instant by `scan_ms` before every scan, so a timer enabled
throughout an advance accumulates `~scan_ms` per scan while edges and
counters stay exact per scan. Because a timer's first scan starts it at
~0 ms, a `TON` with `PT := T#10s` and `scan_ms: 10` reaches `Q = TRUE`
after ~1001 scans (elapsed ≈ `(scans - 1) × scan_ms`); keep scenario
margins comfortably away from the PT boundary, as in the samples
(`advance: 9.5s` → not yet, then `advance: 1s` → done).

State transitions keyed on scan counts (like `timer > 100` in
TrafficLight.st) are exact: they fire on precisely the scan where the
condition becomes true.

## Exit codes

| Code | Meaning                                  |
|------|------------------------------------------|
| 0    | check/test passed                        |
| 1    | compile errors / scenario failures       |
| 2    | usage error or unreadable directory/file |

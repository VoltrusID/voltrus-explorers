# Voltrus PLC — Product Roadmap

## Vision

The only native macOS PLC development tool. Not CODESYS — lighter, faster, Mac-native.
IEC 61131-3 Structured Text editor with a built-in softPLC runtime, real I/O drivers,
and AI-assisted programming.

**Positioning:** Write ST code → compile → run on softPLC → talk to real devices. All on your Mac.

---

## v0.1 — MVP

Core editor + compiler + runtime, end-to-end pipeline working.

- [x] ST compiler (`mt-stc`) — parse IEC 61131-3 ST via iecst, emit bytecode
- [x] Bytecode VM runtime (`mt-plc-runtime`) — cyclic scan, stack machine
- [x] CodeMirror 6 editor with IEC ST syntax highlighting
- [x] Compile → Run → Watch variables pipeline
- [x] Watch table with live variable monitoring
- [x] Runtime controls (Run / Stop / Step / scan cycle config)
- [x] Diagnostics panel (compile errors/warnings)
- [x] Build & release infrastructure (DMG, R2, D1, Cloudflare Worker)
- [x] App Store build scripts

---

## v0.2 — Usable Editor + Real I/O

Make it a tool engineers actually want to use daily.

### Editor
- [x] Multi-file tabs with proper save/load (persist to disk)
- [x] Project tree sidebar (folder-based `.st` files)
- [x] **Project persistence** — save/restore full project state (.vltrus files)
- [x] Autocomplete for keywords, types, standard functions
- [x] Go-to-definition for variables and function blocks
- [x] Inline error squiggles (not just bottom panel)
- [x] Dark/light theme toggle working end-to-end

### Runtime
- [x] Timer function blocks (TON, TOF, TP) — real implementations, not stubs
- [x] Counter function blocks (CTU, CTD, CTUD)
- [x] Proper edge detection (R_TRIG, F_TRIG)
- [x] Runtime state persistence across compile (keep variable values)
- [x] **Cycle timing graph** — real-time visualization of scan times
- [x] **Scan-time histogram** — engineers obsess over cycle jitter; show min/max/avg/p99

### I/O — Talk to Real Devices
- [x] Modbus TCP driver (wrap `mt-modbus` via IoDriver trait)
- [x] **EtherNet/IP driver** — huge for Rockwell users migrating to softPLC; wrap `mt-enip`
- [x] I/O mapping panel (variable ↔ Modbus/EIP register/address)
- [x] Direct address support (`%IX0.0`, `%QW100`, `%MW200`)
- [x] I/O connection status indicators
- [x] Read/write test in I/O mapping panel

---

## v0.3 — AI-Assisted PLC Programming

**First PLC IDE with integrated AI. This is the differentiator — our moat.**

### AI Features
- [x] AI chat panel — ask questions about IEC 61131-3, ST syntax, PLC concepts
- [x] AI code generation — describe logic in natural language, get ST code
- [x] AI code explanation — "explain this rung/block" for any selected code
- [x] AI error fix — click on diagnostic, get suggested fix with compiler validation
- [x] AI test case generation — "generate test scenario for this motor control block"
- [x] **Context-aware AI**: inject current POU, variables, I/O map, diagnostics into every prompt
- [x] **Safety-first workflow**: every AI suggestion shows diff → "Validate" button runs compiler + basic simulation tests before applying

### Implementation
- [x] LLM integration — **Claude API** (strong at code generation) + **local Ollama** (offline, security-conscious industrial users)
- [x] System prompt with IEC 61131-3 ST knowledge and project context
- [x] **Full code context injection**: current POU, all variables, I/O mapping, diagnostics
- [x] Streaming responses in sidebar panel
- [x] **Diff-based code application**: accept/reject individual hunks, preview full change
- [x] **Pre-apply validation**: compiler check + simulation smoke test before code lands

### Also in v0.3
- [x] Multiple POU support (PROGRAM, FUNCTION_BLOCK, FUNCTION)
- [x] STRUCT and ENUM type definitions
- [x] Standard library functions (math, string, conversion, bitwise, selection — 50+ total)
- [x] Array support (ARRAY [1..10] OF INT)
- [x] Example project templates (motor control, batch process, traffic light)
- [x] String literals and operations (CONCAT, LEN, LEFT, RIGHT, MID, FIND, etc.)
- [x] Bitwise operations (SHL, SHR, ROL, ROR)
- [x] WORD/DWORD type conversions

---

## v0.4 — Professional Tooling ✅ Complete

For engineers using it in production environments.

### Debugging
- [x] Breakpoints (pause at specific ST lines)
- [x] Single-step with variable inspection
- [x] Call stack view
- [x] Force/unforce variables from watch table
- [x] **Trend chart** — plot variable values over time; multi-series, pan/zoom

### Connectivity
- [x] OPC-UA driver (wrap `mt-opcua`)
- [x] Siemens S7 driver (wrap `mt-s7`)
- [x] MQTT driver for IoT bridges
- [x] Multi-driver I/O (mix Modbus + EtherNet/IP + OPC-UA in one program)
- [x] Connection profiles (save/restore I/O configurations via .vltrus)
- [x] **Share I/O configs with Voltrus SCADA** — JSON export/import or direct link for unified toolchain

### Project Management
- [x] `.voltrus` project file format
- [x] Import/export ST programs
- [x] **Version control integration** — git diff for .st files, huge for teams
- [x] **Cross-reference table** — who reads/writes each variable; find usage across all POUs
- [x] Project-wide search and replace

---
## Key Directories

| Component | Path |
|-----------|------|
| PLC IDE (this project) | `/Users/macbookpro/Documents/JoyoDigitama/MacTools/apps/voltrus-plc` |
| HMI Designer | `/Users/macbookpro/Documents/JoyoDigitama/MacTools/apps/voltrus-hmi` |
| SCADA backend | `/Users/macbookpro/Documents/JoyoDigitama/voltrus/voltrus` |
| ESP32 firmware | `/Users/macbookpro/Documents/JoyoDigitama/MacTools/firmware/esp32-tester` |
| Landing page (main) | `/Users/macbookpro/Documents/JoyoDigitama/voltrus/voltrus-landing` |
| Landing page (PLC) | `/Users/macbookpro/Documents/JoyoDigitama/voltrus/voltrus-landing/voltrus-plc` |
| Shared Rust crates | `/Users/macbookpro/Documents/JoyoDigitama/MacTools/crates` |
| MacTools root | `/Users/macbookpro/Documents/JoyoDigitama/MacTools` |

---
## Tech Debt Priorities (Re-ranked)

These unlock premium feel and conversion:

1. ~~**Custom Lezer grammar for IEC ST**~~ ✅ — LRLanguage with proper Lezer grammar, syntax highlighting, autocomplete
2. ~~**WASM build target for web demo**~~ ✅ — 331KB WASM binary, mt-stc-wasm crate with JS bindings, live demo on landing page
3. ~~**Fix iecst false-positive "unused variable" warnings**~~ ✅ — filtered in both compile paths
4. ~~**Test coverage (backend)**~~ ✅ — 56 tests (19 compiler + 37 VM): structs, enums, CTUD, direct address, persistence, arrays, strings, math, bitwise, CASE, EXIT/CONTINUE
5. ~~**Improve codegen error messages with source location**~~ ✅ — push_error() helper, all failure points report line/col
6. ~~**Performance**: profile VM for high-frequency scan cycles~~ ✅ — 3.17M cycles/sec (0.3μs/cycle) in release, 612K/sec in debug. Well under 1ms target.
7. ~~**Frontend test coverage**~~ ✅ — 23 unit tests (Vitest) + E2E tests (Playwright) for store, editor, compile-run-watch flows
8. ~~**Persistent WebSocket command channel**~~ ✅ — split sink/stream, stored in TargetState
9. ~~**Shared PROTOCOLS constant**~~ ✅ — extracted to `src/constants/protocols.ts` with consistent labels and optional defaultPort
10. ~~**Cross-reference read detection bug**~~ ✅ — fixed LHS/RHS detection: reads on RHS of assignments now correctly detected, declarations no longer match `:=` lines

## v1.0 — Production Release

Ready for App Store, ready for real work.

- [x] Full IEC 61131-3 ST compliance (all statements, all types) — CASE with value comparison + ranges, EXIT/CONTINUE, FOR/WHILE/REPEAT, IF/ELSIF/ELSE
- [x] Comprehensive standard function library — 50+ functions: math, string, conversion, bitwise, selection
- [x] Codegen error messages with source locations — line/col for unknown vars, unknown functions, bad member access
- [x] Hardware-in-the-loop testing support (ESP32-S3 target via Modbus TCP) — sample program + IoPanel quick presets
- [x] License activation (Polar.sh via Cloudflare Worker, LicensePanel UI, activate/validate/deactivate)
- [x] Auto-update (Tauri v2 updater plugin, check/install/relaunch)
- [x] User documentation and tutorials — ST language reference at voltrus.id/voltrus-plc/docs.html
- [ ] App Store submission (deferred — using direct download via CF Worker for now)
- [x] Landing page with interactive WASM compiler demo (voltrus.id/voltrus-plc/)
- [x] Direct download via Cloudflare Worker — dl-voltrus-plc.voltrus.id, R2 + D1, auto-update endpoint

---

## v1.0.1 — Stability & UX Polish ✅ Complete

Bug fixes and UX improvements from comprehensive code/UI/flow review.

- [x] **target-disconnected listener** — frontend now detects WebSocket disconnect and resets state (targetConnected, executionMode, runtimeState)
- [x] **Mode-aware toolbar controls** — Run/Stop/Single Scan dispatch to local or remote runtime based on execution mode
- [x] **Mode switch stops active runtime** — switching Local↔Remote stops the running runtime to prevent silent background execution
- [x] **Primary button contrast** — changed from `color: #000` to `#fff` for proper readability on green accent background
- [x] **Toolbar overflow** — added `overflow-x: auto` so buttons don't clip at narrow window widths
- [x] **Duplicate CSS fix** — removed redundant `border-right` in `.editor-tab`
- [x] **Disambiguated Step buttons** — renamed runtime "Step" to "Single Scan" to distinguish from debug "Step Into"
- [x] **Modal accessibility** — added `role="dialog"`, `aria-modal`, `aria-labelledby`; Escape key dismisses modals
- [x] **Scan cycle input safety** — added `min={1}` and `max={60000}` to prevent zero/negative/extreme values
- [x] **Connect loading state** — Target Connect button shows "Connecting..." and disables during attempt
- [x] **Unbounded log array** — capped at 1000 entries to prevent memory growth
- [x] **ARIA labels** — added `aria-label` to theme toggle and tab close buttons
- [x] **Project deserialization** — always resets `executionMode` to `"local"` (can't restore live connections)
- [x] **Log array capping** — ring buffer approach prevents unbounded memory growth in long sessions

---

## v1.1 — Voltrus Runtime (Standalone PLC Execution Engine)

**The critical evolution: IDE and runtime are separate processes.**

The IDE includes an embedded softPLC for **offline simulation** — engineers can develop and test
without any hardware. v1.1 adds a standalone runtime that runs on real hardware. The IDE becomes
a deploy/debug tool, and the runtime runs 24/7 on the target device.

### Three Value Propositions

| Tier | What | Target Customer |
|------|------|-----------------|
| **ESP32 runtime** | $5 PLC for simple control | Education, IoT, small automation, hobbyists |
| **Linux runtime** (RevPi, RPi, IPC) | SoftPLC for real plants | Small-medium factories, retrofit projects |
| **IDE data access to any PLC** | Read/write variables via S7, Modbus, OPC-UA | Engineers with existing vendor-locked hardware |

### System Architecture

```
                         ┌─────────────────────────────┐
                         │   Voltrus PLC IDE (macOS)    │
                         │   [compiler + editor + AI]    │
                         └──────┬──────────┬────────────┘
                                │          │
                    ┌───────────┘          └──────────────┐
                    │ gRPC (tonic)                        │ HTTP POST + WebSocket
                    │ protobuf                            │ FlatBuffers
                    ▼                                     ▼
    ┌──────────────────────────────┐    ┌────────────────────────────────┐
    │  Linux Runtime               │    │  ESP32-S3 Runtime              │
    │  ──────────────────────────  │    │  ──────────────────────────    │
    │  Industrial PC / x86_64      │    │  240MHz dual-core, PSRAM       │
    │  RevPi Core (RPi CM4)        │    │  WiFi + GPIO + RS485           │
    │  Raspberry Pi 4/5            │    │  Protocol stacks in firmware   │
    │  Wago PFC200 Gen 2 (Linux)   │    │  Physical I/O on GPIO pins     │
    │  UniPi Neuron                │    │  ~$5 hardware                   │
    └──────────┬───────────────────┘    └──────────┬─────────────────────┘
               │                                    │
               │         ┌───────────────┐          │
               └────────►│  Voltrus      │◄─────────┘
                 gRPC    │  SCADA        │  WebSocket + FlatBuffers
                         │  (Axum)       │
                         └───┬───────────┘
                             │
                 ┌───────────┼───────────────────────┐
                 │           │                       │
                 ▼           ▼                       ▼
          ┌──────────┐ ┌──────────┐          ┌──────────────┐
          │ S7 PLC   │ │ Modbus   │          │ Any vendor-  │
          │ (S7comm) │ │ devices  │          │ locked PLC   │
          └──────────┘ └──────────┘          │ via OPC-UA / │
                                             │ Modbus / etc │
          Data access only (not runtime)     └──────────────┘
```

**The runtime is the hub for Voltrus targets.** Both the IDE and SCADA connect to it as clients.
For vendor-locked PLCs (S7, Codesys devices), the SCADA talks directly to the PLC via standard
protocols — no Voltrus runtime needed. The IDE can also monitor vendor PLCs directly for convenience,
but the MacTools mini-tools (Modbus Poll, S7 Explorer, etc.) serve that purpose too.

### Transport Protocol Decision

ESP32 cannot run gRPC (requires HTTP/2, 2-4MB RAM — no ESP-IDF port exists). Different transports
are used per target, with the same logical message types across both.

| Aspect | Linux Runtime | ESP32-S3 Runtime |
|--------|--------------|------------------|
| **Transport** | gRPC (tonic) | WebSocket (binary frames) |
| **Serialization** | protobuf (prost) | FlatBuffers (zero-copy) |
| **Bytecode deploy** | gRPC `Deploy()` RPC | HTTP POST `/deploy` |
| **Variable streaming** | gRPC server-streaming | WebSocket push (FlatBuffer) |
| **Write commands** | gRPC `ForceVariable()` | WebSocket (reverse direction) |
| **Discovery** | mDNS (`_voltrus-runtime._tcp`) | mDNS or manual IP |
| **RAM overhead** | Negligible (full Linux) | ~10 KB (FlatBuffers + WS buffer) |

**Why FlatBuffers for ESP32:** Zero-copy deserialization — the ESP32 writes variable values directly
into the buffer that gets sent over WebSocket. No intermediate allocation. ~5KB RAM. The SCADA reads
the same FlatBuffer on the other end with the Rust `flatbuffers` crate.

**Why gRPC for Linux:** Strongly typed, code-gen from `.proto`, bidirectional streaming, Tokio-native.
Both IDE and SCADA use the same `tonic` client.

### ESP32 Memory Budget

| Component | Internal SRAM | PSRAM |
|-----------|--------------|-------|
| WiFi stack | ~60 KB | — |
| FreeRTOS + system | ~30 KB | — |
| PLC VM + variable memory | ~30-50 KB | — |
| WebSocket client | ~4 KB | ~8 KB buffers |
| HTTP server (bytecode upload) | ~8 KB | — |
| FlatBuffers builder | ~5 KB | ~4 KB buffer |
| **Total** | **~140-160 KB** | **~12 KB** |
| **Remaining** | **~140 KB** | **~7.9 MB** |

Ample headroom for the VM, protocol handlers, and application logic.

### Linux Hardware Targets

Platforms where Voltrus Runtime can be installed as a Linux service (replacing or alongside Codesys):

| Target | Hardware | OS | Openness |
|--------|----------|-----|----------|
| **RevPi Core/Connect** | RPi CM4, DIN rail, -40°C to 55°C | Raspbian Linux | Fully open — run any binary, Docker |
| **Raspberry Pi 4/5** | ARM64 | Raspberry Pi OS | Fully open |
| **Generic industrial PC** | x86_64 | Any Linux | Fully open |
| **Wago PFC200 Gen 2** | ARM Cortex-A8 | RT Linux (WAGO-Linux) | SSH access, Docker on newer models |
| **UniPi Neuron/Patron** | RPi CM, integrated relays + analog I/O | Linux | Fully open, built-in Modbus |

### Vendor-Locked PLCs (Data Access Only)

These PLCs run proprietary firmware — no third-party program deployment possible.
The SCADA talks to them directly via standard protocols:

| PLC Family | Data Access Protocol | Program Deployment |
|------------|---------------------|-------------------|
| Siemens S7-1200/1500 | S7comm (snap7 / `mt-s7`) | No — TIA Portal only |
| Codesys PLCs (Festo, ifm, ABB, etc.) | OPC-UA, Modbus TCP | No — Codesys gateway only (port 1217, proprietary) |
| Beckhoff TwinCAT | ADS protocol, OPC-UA | No — TwinCAT XAE only |
| Rockwell ControlLogix | EtherNet/IP CIP (`mt-enip`) | No — Studio 5000 only |
| Omron NJ/NX | OPC-UA, Modbus | No — Sysmac Studio only |

The IDE's I/O drivers (Modbus, S7, ENIP, OPC-UA, MQTT) let engineers monitor these PLCs directly
for convenience, but the primary monitoring tooling is the MacTools suite (sold separately).

### Phase 1: Runtime Core (`crates/mt-plc-runtime` refactor) ✅ Complete

Extract the bytecode VM from the IDE into a standalone-capable library.

- [x] **Separate VM from Tauri** — the VM must work without Tauri/JS bridge; pure Rust, no UI dependencies
- [x] **Bytecode bundle format** — self-contained deployable unit:
  ```
  [4B magic "VLT1"] [2B version] [4B instruction count] [4B variable count]
  [4B I/O mapping count] [instructions...] [variable init values...] [I/O mappings...]
  [4B CRC32]
  ```
- [x] **Shared message types** — define a schema that works for both gRPC (protobuf) and ESP32 (FlatBuffers):
  - `RuntimeState` — stopped/running/paused/error, cycle count, scan time stats
  - `VariableUpdate` — variable name, type, value (variant), quality, timestamp
  - `DeployRequest` — bytecode bundle, config overrides
  - `ForceRequest` / `ForceResponse` — variable override commands
  - `IoConfig` / `IoStatus` — I/O driver configuration
- [x] **gRPC service definition** — `crates/mt-grpc/` with protobuf:
  ```protobuf
  service PlcRuntime {
    rpc Deploy(DeployRequest) returns (DeployResponse);
    rpc GetRuntimeState(Empty) returns (RuntimeState);
    rpc Start(StartConfig) returns (RuntimeState);
    rpc Stop(Empty) returns (RuntimeState);
    rpc Step(Empty) returns (RuntimeState);
    rpc WatchVariables(WatchRequest) returns (stream VariableUpdate);
    rpc ForceVariable(ForceRequest) returns (ForceResponse);
    rpc UnforceVariable(UnforceRequest) returns (ForceResponse);
    rpc GetIoStatus(Empty) returns (IoStatus);
    rpc ConfigureIo(IoConfig) returns (IoStatus);
  }
  ```
- [x] **gRPC server** — tonic server wrapping the VM, runs as Linux systemd service
- [x] **gRPC client** — shared client crate for IDE and SCADA
- [ ] ~~**FlatBuffers schema**~~ — replaced by custom zero-alloc binary encoder in ESP32 firmware (`plc_flatbuf.h/c`), serving the same purpose with lower overhead

### Phase 2: IDE Integration (Deploy & Remote Debug) — Core Complete

- [x] **Bytecode serializer** — `serialize_bytecode()` in `mt-plc-types` converts `CompiledModule` to ESP32 binary format (header + instructions + variables + I/O mappings + CRC32)
- [x] **Target selection panel** — "Target" tab in right sidebar, manual IP input with quick presets, status LED indicator
- [x] **Deploy button** — "Build & Deploy" compiles current project → serializes bytecode → HTTP POST to `http://<ip>/api/deploy`
  - ESP32 target: HTTP POST bytecode binary
- [x] **Remote runtime controls** — Run/Stop/Step buttons control the remote ESP32 runtime via WebSocket binary commands
- [x] **Remote variable watching** — WebSocket reader task streams `watch-update` events into existing WatchTable (zero UI changes needed)
- [x] **Execution mode toggle** — Local/Remote switch in target panel, local VM for offline simulation
- [x] **SCADA registration** — one-click "Register in SCADA" button sends device + tags to SCADA REST API
- [x] **ESP32 REST endpoints** — `GET /api/health`, `/api/variables`, `/api/state`, `POST /api/variables/<name>` for SCADA polling compatibility
- [x] **Mode-aware toolbar controls** — Run/Stop/Step dispatch to local or remote runtime based on execution mode; mode switch stops active runtime
- [x] **Target disconnect detection** — frontend listens for `target-disconnected` events, resets state on network loss
- [x] **Connect loading state** — Connect button shows "Connecting..." and disables during connection attempt
- [x] **Remote breakpoints** — set/clear breakpoints on remote runtime (Linux only — ESP32 has limited debug)
- [x] **Target discovery** — mDNS/Bonjour `_voltrus-runtime._tcp` service advertisement
- [x] **Linux target support** — gRPC `Deploy()` RPC for Linux runtime deployment
- [x] **Persistent WS command channel** — reuse streaming WebSocket for commands instead of opening short-lived connections

### Phase 3: ESP32 Firmware (`firmware/esp32-tester` extension) ✅ Complete

Add a "PLC Runtime" mode to the existing ESP32-S3 protocol tester firmware.
The existing firmware already has all protocol stacks (Modbus, S7, ENIP, MQTT, OPC-UA, BACnet, DNP3).

- [x] **Bytecode receiver** — HTTP POST `/api/deploy` endpoint, stores bytecode in SPIFFS/flash partition
- [x] **VM port to C** — port `mt-plc-runtime` bytecode interpreter (60+ opcodes) to C for ESP-IDF integration
  - Full instruction set: LOAD, STORE, ADD, SUB, MUL, DIV, MOD, AND, OR, XOR, NOT, JMP, JZ, JNZ, CALL, RET, CMP, NOP, SHL, SHR, ROL, ROR, TON, TOF, TP, CTU, CTD, CTUD, R_TRIG, F_TRIG, MUX, SEL, MOVE, LIMIT, ABS, SQRT, etc.
  - Type-preserving arithmetic (INT+INT→INT, promote to REAL only when needed)
  - Watchdog timer (100K instruction limit per cycle) prevents infinite loops
- [x] **FlatBuffers schema + C codegen** — custom zero-allocation binary encoder for variable streaming (no FlatBuffers library dependency)
  - `plc_flatbuf.h/c` — encode/decode helpers for variable snapshots, status, force/unforce/start commands
  - Little-endian read/write helpers, frame size up to 8192 bytes
- [x] **WebSocket server** — `esp_http_server` with WebSocket upgrade, handles variable streaming + commands
  - Message types: variable snapshot (0x01), force (0x02), unforce (0x03), start (0x04), stop (0x05), step (0x06), status (0x07), error (0x08)
  - Thread-safe force/unforce via runtime mutex (no raw VM pointer exposure to WS handlers)
- [x] **Scan cycle executor** — FreeRTOS task on Core 1 running VM in tight loop:
  ```c
  void plc_scan_task(void *arg) {
      while (running) {
          xSemaphoreTakeRecursive(mutex, portMAX_DELAY);
          plc_io_bridge_read_inputs(&vm);
          plc_vm_execute_cycle(&vm);
          plc_io_bridge_write_outputs(&vm);
          xSemaphoreGiveRecursive(mutex);
          plc_ws_push_variables(&vm);
          vTaskDelayUntil(&last, cycle_ticks);
      }
  }
  ```
  - FreeRTOS recursive mutex protects all shared VM state between scan task (Core 1) and HTTP/WS handlers (Core 0)
  - Breakpoint/pause support with state transition to PLC_STATE_PAUSED
- [x] **I/O bridge** — connect VM variable addresses to protocol handlers and GPIO:
  - `%QX0.0` → Modbus coil write
  - `%IX0.0` → Modbus discrete input read
  - `%QW100` → S7 DB write
  - Physical GPIO → relay/LED/buzzer control (already in firmware)
- [x] **Bytecode validation** — CRC32 (hardware-accelerated `esp_crc32_le`), bounds checking (`CHECK_BOUNDS` macro), magic/version/count validation
- [x] **menuconfig integration** — add PLC options: scan cycle time, variable allocation, I/O mappings, WiFi config
- [x] **mDNS advertisement** — broadcast `_voltrus-runtime._tcp` so IDE/SCADA auto-discover the device

### Phase 4: Voltrus SCADA Integration ✅ Core Complete

Connect the SCADA backend to Voltrus devices as native data sources, alongside existing vendor PLCs.

- [x] **"Voltrus Runtime" protocol driver** in SCADA — treats runtime like any other device
  - Linux runtime: connects via gRPC client (`voltrus_plc::client::PlcRuntimeClient`)
  - ESP32 runtime: connects via WebSocket + FlatBuffers (same logical API)
  - Both expose the same logical API (variable browse, subscribe, write)
- [x] **Auto-discovery** — mDNS scan finds Voltrus Runtime instances on the network
- [x] **Variable browsing** — SCADA reads available variables, types, and I/O mappings from runtime
- [x] **Live data subscription** — subscribe to variable streams via gRPC `WatchVariables` (broadcast channel), populate dashboards and alarms
- [x] **Write-back** — SCADA writes setpoints/overrides to runtime variables via `ForceVariable` (HMI-style control)
- [x] **Deep links** — "Open in PLC Editor" from SCADA launches IDE with the project loaded
- [x] **Unified device list** — SCADA shows Voltrus runtimes alongside S7, Modbus, OPC-UA devices
  - `VoltrusPlcDevice` model with tags, runtime state, scan stats
  - REST API endpoints for device CRUD and tag management
  - Poller manager integration for automatic data collection
  - Storage layer with Diesel ORM (migration + repository)

### Crate Structure

```
crates/
├── mt-stc/              # ST compiler (unchanged)
├── mt-plc-types/        # Shared PLC types (unchanged)
├── mt-plc-runtime/      # Bytecode VM — refactored to standalone (no Tauri dep)
├── mt-grpc/             # NEW — protobuf defs + gRPC server (Linux) + gRPC client
├── mt-flatbuf/          # NEW — FlatBuffers schemas + Rust codegen (ESP32 transport)
└── ...

firmware/
└── esp32-tester/
    └── components/
        └── plc_runtime/  # NEW — ESP-IDF C component: VM + FlatBuffers + WS server + I/O bridge
```

### Dependencies

| Crate | Purpose | Target |
|-------|---------|--------|
| `tonic` + `prost` + `tonic-build` | gRPC framework + protobuf codegen | Linux runtime, IDE client, SCADA client |
| `flatbuffers` | Zero-copy serialization for ESP32 | IDE ESP32 client, SCADA ESP32 client |
| `tokio` | Async runtime (already in workspace) | All Rust components |
| `mdns-sd` | mDNS service discovery | IDE, SCADA |

### Build Order (for implementing agent)

1. ~~**`crates/mt-plc-runtime` refactor** — extract VM core from Tauri, pure Rust, no UI deps~~ ✅
2. ~~**Bytecode bundle format** — define and implement the deployable binary format~~ ✅
3. ~~**`crates/mt-grpc/`** — proto definitions + build.rs + gRPC server + gRPC client~~ ✅
4. ~~**`crates/mt-flatbuf/`** — FlatBuffers schema + Rust codegen for ESP32 transport~~ ✅ (custom zero-alloc encoder)
5. ~~**Linux runtime binary** — standalone `voltrus-runtime` crate wrapping VM + gRPC server + I/O~~ ✅
6. ~~**IDE target panel** — React UI + Tauri commands for target selection, deploy, remote control~~ ✅
7. ~~**ESP32 `plc_runtime` component** — C VM port + FlatBuffers + WebSocket server + I/O bridge~~ ✅
8. ~~**SCADA Voltrus driver** — gRPC client + FlatBuffers client as SCADA data source~~ ✅

---

## v1.2 — HMI (Human-Machine Interface)

**The operator's control panel.** A standalone Tauri app (`apps/voltrus-hmi/`) for designing and deploying touch-optimized dashboards.

### What It Is

A **separate Tauri desktop app** (like voltrus-plc) that lets engineers design HMI screens — drag-and-drop widgets, bind to PLC variables, deploy to target devices. The output is a **self-contained HTML/JS bundle** that runs in any browser.

**NOT a competitor to Weintek/Kinco/Delta.** Traditional HMIs run proprietary firmware and only load their own screen formats. Voltrus HMI targets **browser-capable displays** — tablets, panel PCs, Raspberry Pi + touchscreen.

### Architecture

```
┌──────────────────────────────┐
│  Voltrus HMI Designer (macOS) │  Separate Tauri app
│  ─────────────────────────── │  - Drag-and-drop widget editor
│  - Widget palette (gauges,   │  - Variable browser (connects to
│    charts, buttons, text,    │    PLC runtime via WS)
│    indicators, sliders)      │  - Deploy to target
│  - Data binding config       │  - Export standalone HTML
│  - Screen navigation editor  │
└──────────┬───────────────────┘
           │ "Build HMI" → generates static HTML/JS/CSS bundle
           ▼
┌──────────────────────────────┐
│  Deploy targets:              │
│  ──────────────               │
│  1. ESP32 SPIFFS              │  HTTP server serves HTML + WS for data
│  2. RPi5 + touchscreen       │  PLC runtime + HMI on same device
│  3. SCADA static dir          │  SCADA serves HMI + relays data
│  4. Standalone HTML file      │  Open in any browser, connect to PLC
└──────────────────────────────┘
```

**Data path for each target:**

| Target | PLC runs on | HMI served by | Data transfer |
|--------|-------------|---------------|---------------|
| ESP32 + tablet | ESP32-S3 | ESP32 `esp_http_server` (SPIFFS) | Browser → WS → ESP32 `/ws/plc` |
| RPi5 + touchscreen | RPi5 (Linux runtime, gRPC) | RPi5 localhost | Browser → WS → SCADA relay → gRPC |
| SCADA server | Any | SCADA web server | Browser → WS → SCADA → PLC |
| Standalone file | Any | Just open the file | Browser → WS → PLC IP hardcoded |

### Hardware Recommendations for Customers

| Tier | Hardware | Cost | Use case |
|------|----------|------|----------|
| **$5** | ESP32-S3 + phone/tablet | $5 + existing | Education, IoT, commissioning |
| **$200** | RPi 5 + 7" industrial touchscreen (DIN mount) | ~$200 | Small installations, **one device = PLC + HMI** |
| **$800+** | Industrial Panel PC (Advantech TPC, OnLogic Helix) | $800–2000 | Real plants, IP65, -20°C to 60°C, 24/7 |
| **Retrofit** | ESP32-S3 + existing traditional HMI (Weintek, etc.) | $5 | ESP32 speaks Modbus RTU slave → existing HMI polls via RS485 |

**RPi5 is the sweet spot.** One device runs the PLC runtime (Linux gRPC service), serves the HMI HTML, and displays it in Chromium kiosk mode. One cable, one device.

**ESP32 is for distributed/headless nodes** — cheap, separate from the display, talks to SCADA or HMI over WiFi.

### Traditional HMI Compatibility

Traditional HMIs (Weintek, Kinco, Delta, Siemens KTP) run proprietary firmware. They can only load screens designed in their own software. Voltrus HMI Designer cannot target them.

**Bridge strategy:** ESP32 firmware already has **Modbus RTU slave** mode. When PLC variables are mapped to Modbus addresses via the I/O bridge, any third-party HMI can poll them via RS485. No Voltrus software needed on the HMI side — just configure Modbus register addresses.

### Product: Voltrus HMI Kit

Pre-configured hardware kit for customers who want "plug and play":
- RPi 5 + 7" industrial touchscreen + DIN rail enclosure
- Pre-loaded with Chromium kiosk mode + Voltrus PLC Runtime
- Power on → enter ESP32 IP or run standalone → done
- **Potential price: $199–299**

### Features

- [x] **Touch-optimized dashboard builder** — drag-and-drop widgets: gauges, charts, buttons, indicators, text displays, sliders
- [x] **Variable browser** — connect to PLC runtime (ESP32 or Linux), discover available variables and types
- [x] **Real-time data binding** — widgets auto-update via WebSocket subscription
- [x] **Write-back controls** — buttons, setpoint inputs, toggle switches that write PLC variables
- [x] **Screen navigation** — multi-page HMI with navigation buttons (home, detail, alarms, settings)
- [x] **Alarm banner** — persistent top bar showing active alarms with acknowledgment workflow
- [x] **Template system** — pre-built HMI templates for common scenarios:
  - [x] Motor control (start/stop, speed, current, overload status)
  - [x] Tank level (level gauge, fill/drain valves, alarms)
  - [x] Conveyor (speed, jam detection, zone status)
  - [x] Temperature control (setpoint, actual, heater/cooler status)
- [x] **Deploy to ESP32** — upload HTML bundle to ESP32 SPIFFS via HTTP POST
- [x] **Deploy to RPi5** — HTTP agent (`mt-hmi-agent`) on target device
- [x] **Standalone HTML export** — self-contained file, embeds WS client with configurable PLC IP
- [x] **Kiosk mode** — full-screen, hide browser chrome, auto-reconnect, watchdog timer
- [x] **User roles** — operator (view + basic control), supervisor (all controls), viewer (read-only)
- [x] **Multi-language** — operator-facing labels in local language, configurable per screen

### Integration with Voltrus SCADA

The HMI reuses SCADA infrastructure when available:
- **Widget SDK** (`src/widget_sdk/`) — rendering primitives, data binding, event system
- **Data sources** — same `voltrus_plc_{device}:{variable}` naming
- **WebSocket broadcast** — same real-time data channel
- **Write-back** — same REST API for variable forcing
- HMI can also work **standalone without SCADA** — connects directly to PLC runtime

### Why separate Tauri app (not PWA inside SCADA)

- Same familiar macOS-native experience as voltrus-plc IDE
- Offline design capability (design screens without SCADA running)
- Can connect directly to PLC targets (not just through SCADA)
- Clearer product identity — "Voltrus HMI Designer" is a standalone product
- Reuses the same Tauri + React + Tailwind stack from voltrus-plc

---

## v1.3 — Quality & Connectivity ✅ Complete

Polish the IDE for daily professional use and improve remote connectivity.

### IDE Quality
- [x] **Frontend tests** — 21 unit tests for store logic (file management, search, cross-reference, undo/redo, I/O mappings, serialization)
- [x] **E2E tests** — Playwright test suite for editor load, toolbar, compile→run→watch flows
- [x] **Undo/redo for project state** — undo stack (max 50) for I/O mappings, watch list, scan cycle, breakpoints; Cmd+Z/Cmd+Shift+Z shortcuts + toolbar buttons
- [x] **Cross-reference table** — find all reads/writes of a variable across all open POUs
- [x] **Project-wide search & replace** — search across all .st files in project, regex support
- [x] **Error recovery improvements** — auto-reconnect to target on network recovery, retry failed deploys, exponential backoff for SCADA registration

### Remote Connectivity
- [x] **Persistent WS command channel** — send start/stop/step/force through the existing streaming WebSocket instead of opening short-lived connections (eliminates race conditions and multi-connection ambiguity)
- [x] **Force/unforce mode awareness** — Rust `force_variable`/`unforce_variable` checks execution mode, not just `target.connected` atomic
- [x] **Target discovery (mDNS)** — IDE scans for `_voltrus-runtime._tcp` on local network, auto-populates target list
- [x] **Linux target support** — gRPC `Deploy()` RPC for deploying to RevPi/RPi/Linux IPC; target kind selector (ESP32/Linux) in TargetPanel
- [x] **Remote breakpoints** — set/clear breakpoints on remote runtime via persistent WS channel (Linux only)

### ESP32 Firmware
- [x] **menuconfig integration** — expose PLC scan cycle, variable allocation, I/O mapping defaults, mDNS/OTA enable as ESP-IDF menuconfig options
- [x] **mDNS advertisement** — broadcast `_voltrus-runtime._tcp` for auto-discovery (guard via `CONFIG_VOLTRUS_PLC_MDNS_ENABLE`)
- [x] **OTA firmware update** — IDE pushes firmware updates to ESP32 via HTTP POST `/api/firmware`, OTA partition table, auto-restart (guard via `CONFIG_VOLTRUS_PLC_OTA_ENABLE`)
- [x] **WiFi configuration portal** — captive portal (SoftAP `Voltrus-Setup`) for initial WiFi setup; NVS credential storage; fallback from failed STA connection

### SCADA Integration
- [x] **Auto-discovery review** — reviewed existing SCADA registration + deep link flow; fixed device_url format, interval_ms semantics, URL validation, targetConnected guard
- [x] **Deep links** — `voltrus-plc://` URL scheme with `scada`, `device`, `ip` params; sets SCADA URL and target IP from SCADA

---

## v2.0 — Graphical Languages

The "be CODESYS" phase — graphical IEC 61131-3 editors alongside ST.

### Function Block Diagram (FBD) ✅ Complete

- [x] React Flow canvas with drag-and-drop from block palette
- [x] Block library: 25+ blocks (logic, arithmetic, comparison, timers, counters, move, bistable, edge detection)
- [x] Custom node components (BlockNode, InputPin, OutputPin, TimerNode, CounterNode)
- [x] Block palette sidebar — grouped by category, draggable cards
- [x] FBD → ST transpiler — topological sort, cycle detection, FB instance management, temp variable generation
- [x] `.fbd` file format — JSON schema stored in OpenFile.content
- [x] EditorRouter — routes `.fbd` files to FBD editor, `.st` to ST editor
- [x] Compile integration — transpiles FBD to ST before feeding to existing compiler
- [x] Unit tests — transpiler, topological sort, block library validation (10 tests)
- [x] New File modal — "FBD DIAGRAM" button

### Future Graphical Languages
- [x] Ladder Diagram (LD) editor — rung editor with contacts/coils/FB blocks, LD→ST transpiler, parallel branch support, 4 sample programs, F-key hotkeys (8 unit tests)
- [x] Sequential Function Chart (SFC) — state machine editor with steps/transitions/OR branching/jumps, SFC→ST transpiler, 4 sample programs, F-key hotkeys (14 unit tests)
- [x] Mixed-language projects (ST + FBD + LD + SFC in same program) — all graphical files transpiled and compiled together
- [x] Simulation mode — interactive I/O panel with toggle inputs, LED outputs, variable monitoring
- [ ] ~~Instruction List (IL)~~ — deprecated in IEC 61131-3 Ed3, skipped

---

## Product Line & Pricing

### Software Products

| Product | Price | What you get | Target customer |
|---------|-------|-------------|----------------|
| **Voltrus PLC IDE** | $29.99 (one-time) | ST editor + compiler + local softPLC + AI assistant + I/O drivers | PLC engineers, automation students |
| **Voltrus SCADA** | Free (up to 5 devices) / $49/mo (unlimited) | Web-based monitoring, dashboards, alarms, historian | Plant supervisors, maintenance teams |
| **Voltrus HMI Designer** | $19.99 (one-time) | Drag-and-drop HMI builder, export standalone HTML | Engineers deploying operator panels |
| **Voltrus Runtime (Linux)** | Free (personal) / $49/device (commercial) | Linux systemd service, gRPC server, runs ST bytecode | RevPi, RPi, industrial PCs |
| **Voltrus Runtime (ESP32)** | **Free** (open firmware) | ESP32-S3 firmware, WebSocket server, Modbus slave | Education, IoT, $5 PLC nodes |
| **MacTools Protocol Suite** | Free | Modbus Poll, S7 Explorer, OPC-UA Explorer, etc. | Engineers with vendor-locked PLCs |

### Hardware (potential)

| Product | Price | What |
|---------|-------|------|
| **Voltrus HMI Kit** | $199–299 | RPi5 + 7" industrial touchscreen + DIN enclosure, pre-loaded with Voltrus Runtime + HMI kiosk |

### Bundles

| Bundle | Price | Includes |
|--------|-------|----------|
| **Voltrus Starter** | $29.99 | PLC IDE only — write ST, simulate locally |
| **Voltrus Complete** | $69.99 | PLC IDE + HMI Designer + SCADA (5 devices) |
| **Voltrus Pro** | $149.99 | Complete + Linux runtime license (3 devices) + priority support |

### Why this pricing works

- **IDE at $29.99** — impulse buy for engineers, 10× cheaper than CODESYS runtime
- **SCADA freemium** — 5 free devices hooks small teams, $49/mo is 1/10th of Ignition/WinCC
- **ESP32 runtime free** — drives adoption in education and IoT, funnel to paid products
- **HMI Designer at $19.99** — accessory purchase, low friction
- **No subscription for IDE** — one-time purchase builds trust, avoids the CODESYS "rent your tools" resentment

### Landing Page & Content Plan

- [ ] **Product page: voltrus.id/voltrus-plc** — interactive WASM demo, features, pricing, download
- [ ] **Blog post: "The $5 PLC: Running IEC 61131-3 on ESP32-S3"** — technical deep dive, viral in embedded community
- [ ] **Blog post: "Why Your PLC IDE Should Be Native macOS"** — positioning against CODESYS Wine hacks
- [ ] **Blog post: "AI-Assisted PLC Programming: First Steps with Voltrus"** — AI differentiation, Claude integration
- [ ] **Blog post: "From ST Code to Factory Floor in 5 Minutes"** — e2e walkthrough: write ST → deploy to ESP32 → monitor in SCADA → HMI
- [ ] **Video: ESP32 + Voltrus PLC demo** — YouTube, 5 min, shows the full e2e flow
- [ ] **Comparison page: Voltrus vs CODESYS vs TIA Portal** — honest feature/price matrix
- [ ] **Blog post: "Deterministic PLC tests: virtual-time acceptance testing in Voltrus"** — angle: "your TON doesn't need 10 real seconds"; publish alongside the mt-plc-cli release
- [ ] **Landing section: "Test your logic in CI"** on voltrus.id/voltrus-plc — `.test.yaml` sample + `voltrus-plc test` capture, once the CLI ships

---

## Competition Landscape

### Comparison Matrix

| Feature | Voltrus PLC | CODESYS | TIA Portal | TwinCAT 3 |
|---------|-------------|---------|------------|------------|
| macOS native | **Yes** | No (Wine) | No | No |
| IDE download size | **~30MB** | ~2GB | ~20GB | ~4GB |
| Price (IDE) | **$29.99** | Free | €2K–€20K+ | Paid (opaque) |
| AI-assisted programming | **v0.3** | No | No | CoAgent (Win only) |
| Modern editor | **CM6** | Legacy | Custom GUI | VS shell |
| Git-friendly | **Plain-text .st** | Binary (+$) | Binary (API) | XML (noisy) |
| IEC 61131-3 languages | **ST + LD + FBD + SFC** (v2.0) | **All 5 + CFC** | All 5 | All 5 + C++ |
| Hardware targets | SoftPLC + **ESP32-S3** + **Linux** (v1.1) | **400+ vendors** | Siemens only | Beckhoff only |
| Runtime license | **Free ESP32** / **$49–149 Linux** | €50–500/device | €1,500+ | Paid (opaque) |
| Multi-platform IDE | **macOS + Win + Linux** | Windows only | Windows only | Windows only |
| SoftPLC included | **Yes — free (simulation)** | Yes | €1,500+ | Yes |
| SCADA integration | **Native (v1.1)** | Via OPC-UA | Via OPC-UA | Via ADS/OPC-UA |
| HMI (operator panels) | **Browser + RPi5/Panel PC (v1.2)** | Via webvisu | Via WinCC | Via Web HMI |
| OPC-UA | v0.4 | Built-in | Built-in | Built-in |

### Key Competitive Intelligence

**CODESYS** (dominant multi-vendor platform):
- IDE is free but Windows-only; runtime costs €50–500 per device
- Major pain points: 15–20 min freezes on incremental compile, multi-monitor broken, binary .project files, undocumented cfg
- CODESYS Go (upcoming) promises Linux IDE + plain-text files — watch closely
- Single CODESYS bug affects all 400+ vendor platforms simultaneously

**TIA Portal** (Siemens, 35–40% market share):
- Engineers call it "the biggest catastrophe in the world" on Siemens' own forum
- Requires "gaming-class" PCs; 5-10 min project open times; frequent crashes
- Total vendor lock-in — Siemens PLCs only
- Openness/DevOps score: **1 out of 9** (worst of all measured platforms)

**TwinCAT 3** (Beckhoff):
- Can BSOD Windows via kernel driver; freezes for minutes on disconnect
- TwinCAT PLC++ (2025): 1.5× faster runtime, new compiler
- CoAgent (2025): AI code gen via Claude/OpenAI — **first mover in AI PLC programming**
- MCP protocol for extensibility; local model support

**Rockwell Studio 5000** (not in table):
- Openness/DevOps score: **0 out of 9** — completely closed
- FactoryTalk Design Workbench (free, late 2025) signals they feel pricing pressure

### Emerging Competitors

| Tool | What It Is | Threat Level |
|------|-----------|-------------|
| **IronPLC** | Rust-based ST compiler + VS Code extension | Low — complementary |
| **B&R Automation Studio Code** | VS Code-based, plain-text ST, Git + Copilot | Medium — best "modern" commercial alternative |
| **MICS** | Browser-native IEC 61131-3 IDE (all 5 languages) | Low now — early, but watch |
| **Xentara** | Software-defined automation, C++/Rust, MCP | Low — different category |
| **OpenPLC v4** | Open-source, cross-platform, full IEC 61131-3 | Low — education/hobbyist |
| **nautilus (Joy Automation)** | Open-source Go "SCADA as software" toolkit: ST/LD/FBD/SFC → own IR/VM, virtual-time YAML acceptance tests, `check` CI gate, online edits with git round-trip, Sparkplug B (TCK-passing), ISA-18.2 alarms, VS Code LSP | Medium — dev-ex credibility (viral ~480-like LinkedIn demo); but a Go toolkit, not a Mac IDE: EIP-only driver, no bare-metal/ESP32, no AI, no SCADA/HMI products. Copy the workflow (tests, CI, diffable artifacts) — being implemented as PLC-4..7 |

### Our Positioning

**Our edge:** macOS native + AI + lightweight. Don't try to be CODESYS. Be the tool Mac engineers reach for first.

**The wedge:** No PLC IDE runs on macOS. Field engineers increasingly carry MacBooks. Students learn on Macs. IoT developers use Macs. CODESYS/TIA/TwinCAT are all Windows-only. Voltrus PLC is the only native option — and we're adding AI before anyone ships it on Mac.

**Risk to monitor:** Beckhoff CoAgent + CODESYS Go could close the AI and Linux gaps respectively. Move fast on v0.2 (I/O) and v0.3 (AI) — the window is 2025–2026.

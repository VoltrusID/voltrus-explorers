# Architecture

The layering follows [muxy](https://github.com/muxy-app/muxy): the app only
draws and sends commands; sessions run in an engine that talks to devices
through the shared protocol crates.

```mermaid
flowchart LR
    APP["vt-app<br/>window, views"] --> UI["vt-ui<br/>design system"]
    APP --> CORE["vt-app-core<br/>workspace, forms, store"]
    APP --> ENGINE["vt-engine<br/>sessions"]
    APP --> PROTO["vt-protocol<br/>messages"]
    CORE --> PROTO
    ENGINE --> PROTO
    ENGINE --> MT["../../crates/mt-*<br/>protocol crates"]
```

| Crate | Role | GPUI |
| --- | --- | --- |
| `vt-protocol` | `Command`s to the engine, `Event`s back, session specs. Plain serde data. | no |
| `vt-engine` | One tokio task per session on its own runtime thread. Polls, writes, simulates, reports. | no |
| `vt-app-core` | Saved connections, tabs, split tree, form validation, fuzzy search, atomic storage. | no |
| `vt-ui` | Theme tokens from a colour scheme, metrics, components, text input, charts. No app state. | yes |
| `vt-app` | The binary: one `AppModel` entity, views as functions over it. | yes |

The allowed dependencies are listed in `Cargo.toml`
(`[workspace.metadata.architecture.allowed-dependencies]`); change that table
and this page together.

## A poll, end to end

```mermaid
sequenceDiagram
    participant View as AppModel
    participant Engine as vt-engine
    participant Session as Session task
    participant Device
    View->>Engine: Open { session, spec }
    Engine->>Session: spawn
    Session-->>View: Points (schema, once)
    loop every poll interval
        Session->>Device: read blocks
        Device-->>Session: registers
        Session-->>View: Values, Traffic, Stats
    end
    View->>Engine: Write { point, value }
    Engine->>Session: control channel
    Session-->>View: Written(result)
```

- The engine owns each session in one task; the app reaches it only through
  the session's control channel, so a slow device never blocks the window.
- The app drains queued events in one batch per wake-up and redraws once.
- Values land in per-point `History` ring buffers (`vt-ui::chart`), shared
  with the charts by `Rc`: painting a frame never copies samples. Each keeps
  the longest trend window (15 min) at the session's reporting rate, between
  1 800 and 45 000 samples, and grows only as samples arrive.
- Trends request a new frame only while their session is live. gpui redraws
  a notified view and its ancestors, so the sidebar, tab strip, inspector,
  status bar and each pane are drawn by their own cached views
  (`vt-app/src/views/cached.rs`, after muxy): an animating trend redraws its
  pane and the thin window shell, while everything else replays its last frame
  until the model changes.
- Lists follow new rows only while scrolled to their end; scrolling back to
  read stays put. Each pane keeps its own scroll positions.
- The engine finishes a closing simulator before opening another on the same
  port, and a simulator takes writes from any master into its simulation and
  holds them, reporting `Locked` so the app shows the point as held.
- Each pane records its width at layout (`AppModel::pane_widths`); the
  header, points columns, trend and traffic rows adapt to it on the next
  frame, so a split pane stays readable on a small screen.

## Why the engine speaks messages

`vt-protocol` is the only contract between the window and the sessions, and
every type in it is serde data (tested to round-trip). Today the engine runs
in-process; the same messages can travel over a socket to a background
engine, as muxy's server does for terminals, so polling and simulators keep
running when the window closes and a phone can attach.

## Adding a protocol

1. Add a `SessionSpec` variant and its spec types in `vt-protocol`.
2. Add a session module in `vt-engine` that turns the protocol crate's API into
   `Points`, `Values`, `Traffic`, `Log` and `Written` events.
3. Add a form to `vt-app-core::forms` (a submodule per protocol, e.g.
   `forms/bacnet.rs`) and its editor in `vt-app` (e.g. `model/bacnet.rs` for
   the fields, `views/editor/bacnet.rs` for the sheet), and map the protocol
   to it in `EditorKind::for_protocol` (the "+" menu lists every protocol that
   has one).

Protocols whose points appear over time (MQTT topics) start with an empty
`Points` and report each new point with `PointsAdded`, which keeps the values
and histories so far; the message behind a value goes in `Payloads` (the
inspector shows it), and `Command::Publish` / `Published` carry free-form
publishes. The MQTT simulator's broker lives in the session task
(`vt-engine/src/mqtt/broker.rs`, on rumqttc's packet codec) because
`mt-mqtt`'s rumqttd-based broker cannot be stopped or rebound in-process.

The points, trend, traffic and log views work for any protocol that reports
points; protocol-specific views (an OPC UA browse tree, an MQTT topic tree)
go next to them in `vt-app/src/views`.

Work that belongs to no session is still a message: the BACnet device picker
sends `Command::DiscoverBacnet` under a fresh id from the session counter, and
the engine answers with one `EventKind::BacnetDevices` under that id, which
the app hands to the open editor instead of a session. The EtherNet/IP picker
does the same with `Command::DiscoverEnip` and `EventKind::EnipDevices`.

Requests that belong to a running session but are no point write go through
its control channel and come back under the session: MQTT's
`Command::Publish` / `Published`, and EtherNet/IP's explicit message,
`Command::CipRequest` / `EventKind::CipReply`, which the inspector's card
(`vt-app/src/views/cip.rs`) sends and shows. The EtherNet/IP adapter
simulator is `mt-enip`'s target (`start_enip_server`) with the plant's values
moved by the session task, plus `answer_list_identity` on the same UDP port
so the picker finds it.

Commands that belong to a protocol but to no point are messages of their own:
a DNP3 master's station commands (integrity and class polls, time sync,
counter freeze, restarts, and controls with an explicit CROB or mode) go as
`Command::Dnp3` and come back as `EventKind::Dnp3Done`; the outstation's
internal indications arrive as `EventKind::Dnp3Iin` whenever they change. A
DNP3 master names its points up front (`Points`) and adds every other point
the outstation reports with `PointsAdded`, like MQTT topics; point flags map to
`Quality` and their names go in `Payloads` for the inspector.
Work a running session does on request, with an answer that is not a value,
is one command and one event per protocol: a NATS client takes
`Command::Nats { session, request }` (publish with headers, request/reply,
list streams, open a stream, fetch from a consumer) and answers with
`EventKind::Nats` (publishes with `Published`). Each waiting request runs on
its own task beside the subscriptions; a request to a session that is not
running is answered "Not connected" at once. The app keeps the replies and
the JetStream browser per connection (`vt-app/src/model/nats.rs`) and draws
them in the inspector (`vt-app/src/views/nats.rs`). The NATS simulator runs
`mt-nats`'s embedded server in the session task (it stops and rebinds
in-process); its request service is an ordinary client of that server.

The serial port picker works the same way: it sends `Command::ListSerialPorts`
under a request id every two seconds while the editor is open (so a device
plugged in shows up), and `EventKind::SerialPorts` comes back to the editor.
Serial sessions carry raw bytes as `SerialData` chunks; the app keeps them as
ready-to-draw text and hex rows (`vt-app/src/model/terminal.rs`), and the
terminal (`vt-app/src/views/terminal.rs`) replaces the traffic view for serial
panes. Sends, macros and logging are `Command::SerialSend` and
`Command::SerialLog`; the app encodes text or hex into bytes, so the engine
writes exactly what it is given. The serial device simulator prints on a
pseudo-terminal from `mt-serial` (`pty::VirtualPort`) and links it into the
temporary folder under a stable name, which is what its monitors open.

## Design system

`vt-ui::theme::Theme` derives every token (surfaces, borders, accent, status
colours, six chart series) from one Ghostty-format colour scheme, so all 492
bundled schemes work. `Voltrus` and `Voltrus Light` are the house themes.
Spacing, type and control sizes come from `theme::metrics`; views never use
raw colours or sizes.

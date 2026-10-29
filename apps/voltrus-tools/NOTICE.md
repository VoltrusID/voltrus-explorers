# Third-party notices

Voltrus Tools is GPL-3.0-or-later (see the repository `LICENSE`). It includes or
adapts the following works under their own licences. The macOS app bundle
carries all of these texts in `Contents/Resources/Licenses`.

| Work | Used for | Licence |
| --- | --- | --- |
| [muxy](https://github.com/muxy-app/muxy), © 2026 Muxy | Crate layering, theme token derivation (`vt-ui/src/theme`), split layout model (`vt-app-core/src/layout.rs`), and the bundled Ghostty-format colour schemes (`vt-ui/assets/themes`, except `Voltrus` and `Voltrus Light`) | MIT |
| [GPUI](https://github.com/zed-industries/zed/tree/main/crates/gpui), Zed Industries, via the [muxy-app/gpui](https://github.com/muxy-app/gpui) fork kept at [VoltrusID/gpui](https://github.com/VoltrusID/gpui) | The UI framework; `vt-ui/src/text_input.rs` follows its `examples/input.rs` | Apache-2.0 (`packaging/licenses/gpui-LICENSE-APACHE.txt`) |
| [Lucide](https://lucide.dev) | Icons in `vt-ui/assets/icons` | ISC (`vt-ui/assets/icons/LICENSE-lucide.txt`) |
| [JetBrains Mono](https://www.jetbrains.com/lp/mono/) | Monospace face for values and addresses | SIL OFL 1.1 (`vt-ui/assets/fonts/OFL-JetBrainsMono.txt`) |
| [iTerm2 Color Schemes](https://github.com/mbadolato/iTerm2-Color-Schemes), © 2011 to present Mark Badolato | Origin of most bundled colour schemes, as distributed with Ghostty | MIT (below) |
| [rumqttc](https://github.com/bytebeamio/rumqtt), Bytebeam | MQTT client (through `mt-mqtt`) and the packet codec of the MQTT broker simulator (`vt-engine/src/mqtt/broker.rs`) | Apache-2.0 (same text as `packaging/licenses/gpui-LICENSE-APACHE.txt`) |
| [async-opcua](https://github.com/FreeOpcUa/async-opcua) 0.18 (`async-opcua-*` crates), the async-opcua and OPC UA for Rust authors | OPC UA client, server and security (through `mt-opcua`); used unmodified from crates.io, where its source is available | MPL-2.0 (`packaging/licenses/async-opcua-LICENSE-MPL-2.0.txt`) |
| [serialport](https://github.com/serialport/serialport-rs) | Serial ports for the serial monitor and Modbus RTU (through `mt-serial` and `mt-modbus`); its source is on [crates.io](https://crates.io/crates/serialport) | MPL-2.0 (`packaging/licenses/serialport-LICENSE-MPL.txt`; the licence text is at https://mozilla.org/MPL/2.0/) |
| [tokio-serial](https://github.com/berkowski/tokio-serial), © 2017 Zac Berkowitz and contributors | Async serial port streams (through `mt-serial`) | MIT (`packaging/licenses/tokio-serial-LICENSE-MIT.txt`) |
| [regex](https://github.com/rust-lang/regex), The Rust Project Developers | The serial monitor's line patterns | Apache-2.0 (same text as `packaging/licenses/gpui-LICENSE-APACHE.txt`) or MIT |
| [async-nats](https://github.com/nats-io/nats.rs) and [nkeys](https://github.com/wasmcloud/nkeys), The NATS Authors / wasmCloud | NATS client (through `mt-nats`, and directly in `vt-engine/src/nats`) and NKey/creds signing | Apache-2.0 (same text as `packaging/licenses/gpui-LICENSE-APACHE.txt`) |
| [btleplug](https://github.com/deviceplug/btleplug), Nonpolynomial (with parts from Rumble and blurmac) | Bluetooth LE central (through `mt-ble`) | BSD-3-Clause, with MIT/Apache-2.0 and BSD-3-Clause parts (`packaging/licenses/btleplug-LICENSE.txt`) |
| [objc2](https://github.com/madsmtm/objc2), Mads Marquart | CoreBluetooth bindings used by btleplug on macOS (`objc2`, `objc2-foundation`, `objc2-core-bluetooth`, `block2`) | MIT (`packaging/licenses/objc2-LICENSE-MIT.txt`) |
| [pyfanuc](https://github.com/diohpix/pyfanuc), diohpix | The FOCAS/Ethernet wire protocol (framing, function numbers, payload layouts) that `mt-focas` implements for the FANUC CNC client and simulator; no code is copied | Unlicense (public domain) |
| [pyfocas](https://pypi.org/project/pyfocas/), © 2026 Moritz Breurather, Rene Schwaiger | The macro write function number and request layout, checked in `mt-focas`; no code is copied | MIT |
| [secsgem](https://github.com/bparzella/secsgem), © 2013-2024 Benjamin Parzella | The SECS-II stream and function layouts, data item meanings, ack codes, GEM state models and HSMS header layout that `mt-secsgem` ports to Rust for the SECS/GEM host and simulator; none of its code is included, and no SEMI standard text is reproduced | LGPL-2.1-or-later, used here under the GPL-3.0-or-later its later-version clause allows (see `crates/mt-secsgem/NOTICE` in the repository) |

The MIT notice required by muxy:

```
MIT License

Copyright (c) 2026 Muxy

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
```

The MIT notice required by iTerm2 Color Schemes (individual themes remain
their authors'):

```
MIT License

Copyright (c) 2011 to Present Mark Badolato

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.

This license covers the iTerm-Color-Schemes repository collection of themes. 

The copyright/license for each individual theme belongs to the author of that theme.
```

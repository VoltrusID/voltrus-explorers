# Vendored crates

Third-party crates patched in through `[patch.crates-io]` (root `Cargo.toml` and `apps/voltrus-tools/Cargo.toml`).
Each is the published crate unchanged except for the dependency lines listed here. Drop a crate from this folder
as soon as upstream publishes a release that makes the change unnecessary.

| Crate | Upstream | Licence | Change | Why |
| --- | --- | --- | --- | --- |
| `rumqttd` 0.20.0 | [bytebeamio/rumqtt](https://github.com/bytebeamio/rumqtt) | Apache-2.0 | `tokio-rustls` 0.25 -> 0.26 (rustls 0.22 -> 0.23), `rustls-webpki` 0.102 -> 0.103.12 | rustls < 0.23.46 and rustls-webpki < 0.103.12 have open advisories (GHSA, Dependabot); 0.20.0 is rumqttd's latest release and its `main` branch is still on rustls 0.22 |
| `rumqttc` 0.25.1 | [bytebeamio/rumqtt](https://github.com/bytebeamio/rumqtt) | Apache-2.0 | `rustls-webpki` 0.102.8 -> 0.103.12 | same webpki advisories; 0.25.1 is the latest release |

The source compiles unchanged against the newer dependencies. `cargo test -p mt-mqtt` covers the broker (TLS included)
and the client; `vt-engine`'s MQTT tests cover the Voltrus Tools side.

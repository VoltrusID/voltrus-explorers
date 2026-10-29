//! MTConnect sessions: a client of an agent's HTTP interface, and a
//! simulated agent (with an optional SHDR adapter) for any MTConnect client.

use serde::{Deserialize, Serialize};

/// The port MTConnect agents usually listen on.
pub const MTCONNECT_PORT: u16 = 5000;

/// The port the agent simulator starts on. Agents usually listen on 5000, but
/// macOS's AirPlay Receiver holds it, so a simulator beside it does not.
pub const MTCONNECT_SIM_PORT: u16 = 5100;

/// The port an SHDR adapter usually listens on.
pub const MTCONNECT_SHDR_PORT: u16 = 7878;

/// A machine the agent simulator plays.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MtconnectTemplate {
    /// A three-axis vertical machining centre.
    #[default]
    Vmc,
    /// A two-axis lathe.
    Lathe,
}

impl MtconnectTemplate {
    pub const ALL: &'static [MtconnectTemplate] = &[Self::Vmc, Self::Lathe];

    pub fn name(self) -> &'static str {
        match self {
            Self::Vmc => "3-axis machining centre",
            Self::Lathe => "2-axis lathe",
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            Self::Vmc => {
                "X, Y and Z axes, spindle speed and load, path feed, part count, controller conditions"
            }
            Self::Lathe => {
                "X and Z axes, spindle speed and load, path feed, part count, controller conditions"
            }
        }
    }
}

/// Watch an MTConnect agent: the device tree becomes points, and the agent's
/// live stream keeps them current.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct MtconnectClientSpec {
    /// The agent's base URL, e.g. `http://localhost:5000`.
    pub url: String,
    /// Only this device (name or UUID); empty watches every device.
    pub device: String,
    /// Time between streamed documents, in milliseconds.
    pub interval_ms: u64,
    /// Request timeout in milliseconds.
    pub timeout_ms: u64,
}

impl Default for MtconnectClientSpec {
    fn default() -> Self {
        Self {
            url: format!("http://localhost:{MTCONNECT_PORT}"),
            device: String::new(),
            interval_ms: 1000,
            timeout_ms: 10_000,
        }
    }
}

/// Play an MTConnect agent on this computer: a simulated machine behind the
/// agent's HTTP interface, optionally with an SHDR adapter port.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct MtconnectAgentSpec {
    /// Address to listen on: `0.0.0.0` for every interface.
    pub bind: String,
    /// HTTP port of the agent.
    pub port: u16,
    pub template: MtconnectTemplate,
    /// Also serve an SHDR adapter, as a machine's adapter would.
    pub shdr: bool,
    pub shdr_port: u16,
}

impl Default for MtconnectAgentSpec {
    fn default() -> Self {
        Self {
            bind: "127.0.0.1".into(),
            port: MTCONNECT_SIM_PORT,
            template: MtconnectTemplate::Vmc,
            shdr: false,
            shdr_port: MTCONNECT_SHDR_PORT,
        }
    }
}

/// `host:port` of an agent URL, without scheme or path.
pub fn mtconnect_host_port(url: &str) -> &str {
    let url = url.trim();
    let rest = url.split_once("://").map_or(url, |(_, rest)| rest);
    rest.split(['/', '?', '#']).next().unwrap_or(rest)
}

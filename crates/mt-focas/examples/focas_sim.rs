//! Run the CNC simulator from a terminal.
//!
//! ```text
//! cargo run -p mt-focas --example focas_sim -- [port] [bind-address] [31i|0i] [--run]
//! ```
//!
//! `--run` starts the demo program cycle and repeats it, so positions, feed and spindle move.
//! Defaults: port 8193, bind address 127.0.0.1, profile 31i-B. Stop with Ctrl-C.

use mt_focas::{CncProfile, FocasSimulator, SimulatorConfig};

#[tokio::main]
async fn main() -> std::io::Result<()> {
    let mut config = SimulatorConfig::default();
    let mut run = false;
    for (i, arg) in std::env::args().skip(1).enumerate() {
        match (i, arg.as_str()) {
            (_, "--run") => run = true,
            (_, "31i") => config.profile = CncProfile::Series31iB,
            (_, "0i") => config.profile = CncProfile::Series0iF,
            (0, port) => {
                config.port = port.parse().expect("port must be a number");
            }
            (1, addr) => config.bind_addr = addr.to_string(),
            (_, other) => panic!("unexpected argument {other}"),
        }
    }
    let sim = FocasSimulator::start(config).await?;
    println!("FOCAS simulator listening on {}", sim.local_addr());
    if run {
        sim.with_model(|m| {
            m.auto_repeat = true;
            m.cycle_start();
        });
    }
    tokio::signal::ctrl_c().await?;
    sim.stop().await;
    Ok(())
}

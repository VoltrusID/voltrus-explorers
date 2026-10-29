//! Run the GEM equipment simulator from a terminal and print every message.
//!
//! ```text
//! cargo run -p mt-secsgem --example gem_sim -- [port] [bind-address] [--active host:port] [--run]
//! ```
//!
//! Passive on 127.0.0.1:5000 (device id 0) by default. `--active host:port` connects to a host
//! instead. `--run` starts a process every time the chamber goes idle, so events and status
//! variables keep moving; every 5 s it also sets or clears the door-open alarm. Stop with Ctrl-C.

use mt_secsgem::sim::{alid, ecid};
use mt_secsgem::{EquipmentSimulator, ProcessState, SimMode, SimulatorConfig};

#[tokio::main]
async fn main() -> std::io::Result<()> {
    let mut config = SimulatorConfig::default();
    let mut run = false;
    let (mut port, mut bind) = (mt_secsgem::DEFAULT_PORT, "127.0.0.1".to_string());
    let mut args = std::env::args().skip(1);
    let mut positional = 0;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--run" => run = true,
            "--active" => {
                let address = args.next().expect("--active needs host:port");
                config.mode = SimMode::Active { address };
            }
            other => {
                match positional {
                    0 => port = other.parse().expect("port must be a number"),
                    1 => bind = other.to_string(),
                    _ => panic!("unexpected argument {other}"),
                }
                positional += 1;
            }
        }
    }
    if let SimMode::Passive { .. } = config.mode {
        config.mode = SimMode::Passive {
            bind_addr: bind,
            port,
        };
    }
    let sim = EquipmentSimulator::start(config).await?;
    match sim.local_addr() {
        Some(addr) => println!("GEM simulator listening on {addr}"),
        None => println!("GEM simulator connecting out"),
    }
    let mut log = sim.subscribe_log();
    tokio::spawn(async move {
        while let Ok(entry) = log.recv().await {
            println!("{}", entry.line());
        }
    });
    if run {
        sim.with_model(|m| {
            m.set_constant(ecid::PROCESS_TIME, 3.0);
        });
        let sim = std::sync::Arc::new(sim);
        let looper = std::sync::Arc::clone(&sim);
        tokio::spawn(async move {
            let mut seconds = 0u64;
            loop {
                tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                seconds += 1;
                looper.with_model(|m| {
                    if m.chamber.state == ProcessState::Idle {
                        let _ = m.start_process(None);
                    }
                    if seconds.is_multiple_of(5) && !m.raise_alarm(alid::DOOR_OPEN) {
                        m.clear_alarm(alid::DOOR_OPEN);
                    }
                });
            }
        });
        tokio::signal::ctrl_c().await?;
        sim.stop().await;
    } else {
        tokio::signal::ctrl_c().await?;
        sim.stop().await;
    }
    Ok(())
}

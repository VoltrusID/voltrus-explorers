//! `Modbus Poll --smoke [file]` and `--smoke-license [file]`: headless self-tests for CI and for checking an installer.
//!
//! It starts the Modbus TCP slave on loopback, connects the master to it, reads a register back and lists
//! the serial ports, then prints one line (to `file` when given, else stdout) and exits with 0 on success.
//! No window is opened, so it needs no display and no WebView2.

use mt_modbus::{Endianness, ModbusClient, ModbusConfig, ModbusTag, RegisterType};
use mt_protocol::ProtocolClient;

/// Returns the one-line report, or why the self-test failed.
pub async fn selftest(version: &str) -> Result<String, String> {
    let port = {
        let probe = tokio::net::TcpListener::bind("127.0.0.1:0").await.map_err(|e| format!("loopback: {e}"))?;
        probe.local_addr().map_err(|e| e.to_string())?.port()
    };
    let (state, _log, shutdown) =
        mt_modbus::start_slave_server(port, "127.0.0.1", 16, 1).await.map_err(|e| format!("slave start: {e}"))?;
    state.lock().map_err(|e| e.to_string())?.registers[3] = 0x1234;

    let client = ModbusClient::connect(ModbusConfig {
        ip: "127.0.0.1".into(),
        port,
        unit_id: 1,
        interval_ms: 1000,
        timeout_ms: 2000,
        endianness: Endianness::Big,
    })
    .await
    .map_err(|e| format!("master connect: {e}"))?;
    let tag = ModbusTag { name: "smoke".into(), address: 3, reg_type: RegisterType::Uint16, scale: 1.0, register_area: "holding".into() };
    let values = client.read(&[tag]).await.map_err(|e| format!("master read: {e}"))?;
    let read = values.first().map(|v| v.as_f64()).unwrap_or(f64::NAN);
    let _ = shutdown.send(true);
    if read != f64::from(0x1234u16) {
        return Err(format!("read {read}, expected {}", 0x1234));
    }

    #[cfg(serial)]
    let serial = match serialport::available_ports() {
        Ok(ports) => format!("{} serial ports", ports.len()),
        Err(e) => return Err(format!("serial enumeration: {e}")),
    };
    #[cfg(not(serial))]
    let serial = "no serial support".to_string();

    Ok(format!(
        "Modbus Poll {version} smoke ok: TCP master and slave on 127.0.0.1:{port} read 0x1234, {serial}, {}-{}",
        std::env::consts::OS,
        std::env::consts::ARCH
    ))
}

/// `--smoke-license`: asks the live licence server about a made-up key, which proves the Windows machine id,
/// TLS and the answer parsing work end to end without spending a real licence.
#[cfg(feature = "keys")]
pub async fn license_check(version: &str) -> Result<String, String> {
    let key: mt_store::key::KeyConfig = serde_json::from_str(include_str!("../keys.json")).map_err(|e| e.to_string())?;
    let dir = std::env::temp_dir().join(format!("modbus-poll-smoke-license-{}", std::process::id()));
    let outcome = mt_store::key::activate(&key, &dir, "VLT-AAAAA-BBBBB-CCCCC", version, false).await;
    let _ = std::fs::remove_dir_all(&dir);
    match outcome {
        Err(e) if e.contains("was not found") => Ok(format!("Modbus Poll {version} licence smoke ok: {} answered that a made-up key is unknown", key.server)),
        Err(e) => Err(e),
        Ok(_) => Err("the licence server accepted a made-up key".into()),
    }
}

#[cfg(not(feature = "keys"))]
pub async fn license_check(_version: &str) -> Result<String, String> {
    Err("this build has no licence keys (built without the `keys` feature)".into())
}

/// Which self-test `--smoke` / `--smoke-license` runs.
#[derive(Clone, Copy)]
pub enum Check {
    Local,
    License,
}

/// Runs the self-test and reports it; the process exit code.
pub fn run(check: Check, version: &str, out: Option<&str>) -> i32 {
    let runtime = match tokio::runtime::Builder::new_multi_thread().enable_all().build() {
        Ok(r) => r,
        Err(e) => return report(out, &format!("smoke FAILED: runtime: {e}"), 1),
    };
    let result = match check {
        Check::Local => runtime.block_on(selftest(version)),
        Check::License => runtime.block_on(license_check(version)),
    };
    match result {
        Ok(line) => report(out, &line, 0),
        Err(e) => report(out, &format!("Modbus Poll {version} smoke FAILED: {e}"), 1),
    }
}

fn report(out: Option<&str>, line: &str, code: i32) -> i32 {
    match out {
        Some(path) => {
            if let Err(e) = std::fs::write(path, format!("{line}\n")) {
                eprintln!("could not write {path}: {e}");
                return 2;
            }
        }
        None => {
            #[cfg(windows)]
            attach_parent_console();
            println!("{line}");
        }
    }
    code
}

/// The release exe is a GUI-subsystem program; stdout only shows when a console is attached.
#[cfg(windows)]
fn attach_parent_console() {
    unsafe extern "system" {
        fn AttachConsole(process_id: u32) -> i32;
    }
    const ATTACH_PARENT_PROCESS: u32 = u32::MAX;
    unsafe {
        AttachConsole(ATTACH_PARENT_PROCESS);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn the_self_test_passes_on_this_machine() {
        let line = selftest("0.0.0").await.unwrap();
        assert!(line.contains("smoke ok"), "{line}");
        assert!(line.contains("read 0x1234"), "{line}");
    }

    #[test]
    fn a_report_goes_to_the_file_when_given() {
        let path = std::env::temp_dir().join(format!("mp-smoke-{}.txt", std::process::id()));
        assert_eq!(run(Check::Local, "9.9.9", Some(path.to_str().unwrap())), 0);
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.starts_with("Modbus Poll 9.9.9 smoke ok"), "{text}");
        let _ = std::fs::remove_file(&path);
    }
}

//! Bluetooth Low Energy on `mt-ble`: scanning, and client sessions that
//! browse a peripheral's GATT services and read, write and subscribe to its
//! characteristics. Both run against the [`central::Central`] trait, so the
//! simulated peripherals of the demo and the tests take the same paths as
//! this computer's radio.

mod central;
pub(crate) mod client;
pub mod decode;
pub mod gatt;
mod simulated;
mod system;

use std::time::Duration;

use tokio::time::Instant;
use vt_protocol::{BleAdapter, BleClientSpec, BleDecoder, BleFormat, BleScan, BleScanSpec};

use central::Central;
use simulated::SimCentral;
use system::SystemCentral;

/// How often a scan reports what it has seen.
const SCAN_REPORT: Duration = Duration::from_millis(400);
/// Longest scan the app may ask for.
const MAX_SCAN: Duration = Duration::from_secs(60);

/// Scan for `spec.duration_ms`, reporting the peripherals seen so far as
/// they appear; the last report is `done`.
pub(crate) async fn scan(spec: BleScanSpec, report: impl Fn(Result<BleScan, String>)) {
    let duration = Duration::from_millis(spec.duration_ms).min(MAX_SCAN);
    match spec.adapter {
        BleAdapter::Simulated => {
            let (central, _notifications) = SimCentral::new();
            scan_with(&central, duration, &report).await;
        }
        BleAdapter::System => match SystemCentral::open().await {
            Ok((central, _notifications)) => scan_with(&central, duration, &report).await,
            Err(error) => report(Err(error)),
        },
    }
}

async fn scan_with<C: Central>(
    central: &C,
    duration: Duration,
    report: &impl Fn(Result<BleScan, String>),
) {
    if let Err(error) = central.start_scan().await {
        report(Err(error));
        return;
    }
    let deadline = Instant::now() + duration;
    let mut ticker = tokio::time::interval(SCAN_REPORT);
    loop {
        ticker.tick().await;
        let done = Instant::now() >= deadline;
        let mut devices = central.peripherals().await;
        // Strongest first; peripherals without a signal reading last.
        devices.sort_by(|a, b| b.rssi.cmp(&a.rssi).then_with(|| a.name.cmp(&b.name)));
        if done {
            let _ = central.stop_scan().await;
        }
        report(Ok(BleScan { devices, done }));
        if done {
            return;
        }
    }
}

/// The clients the BLE demo opens, on the simulated adapter: a heart rate
/// monitor, and an environment sensor whose setpoint decodes as a float32.
pub fn demo_clients() -> Vec<(&'static str, BleClientSpec)> {
    let monitor = BleClientSpec {
        adapter: BleAdapter::Simulated,
        peripheral: simulated::HRM_ID.into(),
        name: simulated::HRM_NAME.into(),
        poll_ms: 5000,
        notify: true,
        decoders: Vec::new(),
    };
    let mut sensor = BleClientSpec {
        adapter: BleAdapter::Simulated,
        peripheral: simulated::ENV_ID.into(),
        name: simulated::ENV_NAME.into(),
        poll_ms: 2000,
        notify: true,
        decoders: Vec::new(),
    };
    sensor.set_decoder(simulated::SETPOINT, BleDecoder::of(BleFormat::F32Le));
    vec![
        (simulated::HRM_NAME, monitor),
        (simulated::ENV_NAME, sensor),
    ]
}

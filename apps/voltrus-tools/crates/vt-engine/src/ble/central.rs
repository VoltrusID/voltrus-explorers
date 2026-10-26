//! The BLE central a session drives: this computer's radio through
//! `mt-ble` ([`super::system`]), or simulated peripherals
//! ([`super::simulated`]) for the demo and the tests. Sessions and scans are
//! written once against this trait.

use mt_ble::BleService;
use vt_protocol::BleDeviceInfo;

/// A value a peripheral pushed (a notification or an indication).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Notification {
    pub peripheral: String,
    /// Full characteristic UUID.
    pub characteristic: String,
    pub data: Vec<u8>,
}

/// One BLE central. Peripherals are named by the identifiers
/// [`Central::peripherals`] reports, characteristics by their full UUIDs.
/// Notifications arrive on the channel handed out with the central.
pub(crate) trait Central: Send + Sync + 'static {
    fn start_scan(&self) -> impl Future<Output = Result<(), String>> + Send;
    fn stop_scan(&self) -> impl Future<Output = Result<(), String>> + Send;
    /// Everything seen since scanning started, with its latest advertisement.
    fn peripherals(&self) -> impl Future<Output = Vec<BleDeviceInfo>> + Send;
    fn connect(&self, peripheral: &str) -> impl Future<Output = Result<(), String>> + Send;
    fn disconnect(&self, peripheral: &str) -> impl Future<Output = Result<(), String>> + Send;
    fn is_connected(&self, peripheral: &str) -> impl Future<Output = Result<bool, String>> + Send;
    /// Discover the GATT services of a connected peripheral.
    fn services(
        &self,
        peripheral: &str,
    ) -> impl Future<Output = Result<Vec<BleService>, String>> + Send;
    fn read(
        &self,
        peripheral: &str,
        characteristic: &str,
    ) -> impl Future<Output = Result<Vec<u8>, String>> + Send;
    fn write(
        &self,
        peripheral: &str,
        characteristic: &str,
        data: &[u8],
    ) -> impl Future<Output = Result<(), String>> + Send;
    fn subscribe(
        &self,
        peripheral: &str,
        characteristic: &str,
    ) -> impl Future<Output = Result<(), String>> + Send;
    fn unsubscribe(
        &self,
        peripheral: &str,
        characteristic: &str,
    ) -> impl Future<Output = Result<(), String>> + Send;
}

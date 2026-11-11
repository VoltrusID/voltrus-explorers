//! An emulated FX2 board for tests: answers the firmware-load request, "re-enumerates" with new
//! USB strings once firmware is written and released from reset, then answers the fx2lafw
//! requests and streams a simulator scenario at the rate `CMD_START` selected.

use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use super::{CMD_GET_FW_VERSION, CMD_GET_REVID_VERSION, CMD_START, CPUCS, EP_DATA, REQ_FIRMWARE_LOAD, UsbBackend, UsbDeviceInfo, UsbLink, rate_of};
use crate::engine::SampleStream;
use crate::error::{LogicError, Result};
use crate::sim::{ScenarioId, SimStream};

/// A stand-in firmware image (the tests never need the real one).
pub fn fake_image() -> Vec<u8> {
    (0..8120u32).map(|i| (i * 7 + 3) as u8).collect()
}

#[derive(Debug)]
pub struct MockState {
    pub present: bool,
    pub firmware_running: bool,
    /// Bumped on every re-enumeration; part of the device id.
    pub generation: u32,
    pub in_reset: bool,
    pub ram_written: usize,
    pub started_rate: Option<u64>,
    pub permission_denied: bool,
    /// The board never re-enumerates after the upload.
    pub stay_gone: bool,
    /// Stop delivering bulk data after this many transfers (USB overrun).
    pub stall_after: Option<usize>,
    pub scenario: ScenarioId,
}

#[derive(Clone)]
pub struct MockFx2(Arc<Mutex<MockState>>);

impl MockFx2 {
    pub fn new(firmware_running: bool) -> Self {
        Self(Arc::new(Mutex::new(MockState {
            present: true,
            firmware_running,
            generation: 1,
            in_reset: false,
            ram_written: 0,
            started_rate: None,
            permission_denied: false,
            stay_gone: false,
            stall_after: None,
            scenario: ScenarioId::I2cSensors,
        })))
    }

    pub fn empty() -> Self {
        let m = Self::new(false);
        m.state().present = false;
        m
    }

    pub fn with_scenario(self, s: ScenarioId) -> Self {
        self.state().scenario = s;
        self
    }

    pub fn state(&self) -> MutexGuard<'_, MockState> {
        self.0.lock().unwrap()
    }
}

impl UsbBackend for MockFx2 {
    fn list(&self) -> Result<Vec<UsbDeviceInfo>> {
        let s = self.state();
        if !s.present || (s.stay_gone && s.generation > 1) {
            return Ok(vec![]);
        }
        let (manufacturer, product) = if s.firmware_running { (Some("sigrok".to_string()), Some("fx2lafw".to_string())) } else { (None, None) };
        Ok(vec![UsbDeviceInfo { id: format!("mock-{}", s.generation), vid: 0x0925, pid: 0x3881, manufacturer, product, serial: None, location: "1-2".into() }])
    }

    fn open(&self, id: &str) -> Result<Box<dyn UsbLink>> {
        let s = self.state();
        if s.permission_denied {
            return Err(LogicError::Permission(super::permission_help().into()));
        }
        if !s.present || id != format!("mock-{}", s.generation) {
            return Err(LogicError::NoDevice);
        }
        Ok(Box::new(MockLink { dev: self.clone(), stream: None, size: 0, delivered: 0, pending: Vec::new() }))
    }
}

struct MockLink {
    dev: MockFx2,
    stream: Option<SimStream>,
    size: usize,
    delivered: usize,
    pending: Vec<u8>,
}

impl UsbLink for MockLink {
    fn control_out(&mut self, request: u8, value: u16, _index: u16, data: &[u8]) -> Result<()> {
        let mut s = self.dev.state();
        match request {
            REQ_FIRMWARE_LOAD if value == CPUCS => {
                let reset = data.first() == Some(&1);
                if s.in_reset && !reset && s.ram_written > 0 {
                    s.firmware_running = true;
                    s.generation += 1;
                }
                s.in_reset = reset;
                Ok(())
            }
            REQ_FIRMWARE_LOAD if s.in_reset => {
                s.ram_written = s.ram_written.max(value as usize + data.len());
                Ok(())
            }
            CMD_START if s.firmware_running && data.len() == 3 => {
                let rate = rate_of([data[0], data[1], data[2]]);
                s.started_rate = Some(rate);
                self.stream = Some(SimStream::new(s.scenario, rate, false)?);
                Ok(())
            }
            _ => Err(LogicError::Usb(format!("request 0x{request:02X} stalled"))),
        }
    }

    fn control_in(&mut self, request: u8, _value: u16, _index: u16, len: u16) -> Result<Vec<u8>> {
        let s = self.dev.state();
        match request {
            CMD_GET_FW_VERSION if s.firmware_running && len >= 2 => Ok(vec![1, 4]),
            CMD_GET_REVID_VERSION if s.firmware_running => Ok(vec![0x04]),
            _ => Err(LogicError::Usb(format!("request 0x{request:02X} stalled"))),
        }
    }

    fn start_bulk(&mut self, ep: u8, size: usize, _transfers: usize) -> Result<()> {
        if ep != EP_DATA {
            return Err(LogicError::Usb("no such endpoint".into()));
        }
        self.size = size;
        Ok(())
    }

    fn read_bulk(&mut self, _timeout: Duration) -> Result<Option<Vec<u8>>> {
        if self.dev.state().stall_after.is_some_and(|n| self.delivered >= n) {
            return Ok(None);
        }
        let Some(st) = self.stream.as_mut() else { return Ok(None) };
        while self.pending.len() < self.size {
            st.read(&mut self.pending)?;
        }
        let buf: Vec<u8> = self.pending.drain(..self.size).collect();
        self.delivered += 1;
        Ok(Some(buf))
    }
}

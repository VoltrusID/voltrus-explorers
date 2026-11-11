//! The real USB backend, on nusb (pure Rust: IOKit on macOS, usbfs on Linux, WinUSB on Windows).
//! **Untested with a board attached**: it compiles and lists devices, nothing more has been run.

use std::time::Duration;

use nusb::MaybeFuture;
use nusb::transfer::{Buffer, Bulk, ControlIn, ControlOut, ControlType, In, Recipient};

use super::{UsbBackend, UsbDeviceInfo, UsbLink, board_for, permission_help};
use crate::error::{LogicError, Result};

const TIMEOUT: Duration = Duration::from_millis(1000);

pub struct NusbBackend;

fn info_of(d: &nusb::DeviceInfo) -> UsbDeviceInfo {
    let location = format!("{}-{}", d.bus_id(), d.port_chain().iter().map(|p| p.to_string()).collect::<Vec<_>>().join("."));
    UsbDeviceInfo {
        id: format!("{:04x}:{:04x}@{:?}", d.vendor_id(), d.product_id(), d.id()),
        vid: d.vendor_id(),
        pid: d.product_id(),
        manufacturer: d.manufacturer_string().map(str::to_string),
        product: d.product_string().map(str::to_string),
        serial: d.serial_number().map(str::to_string),
        location,
    }
}

fn usb_err(what: &str, e: impl std::fmt::Display) -> LogicError {
    LogicError::Usb(format!("{what}: {e}"))
}

fn open_err(e: nusb::Error) -> LogicError {
    match e.kind() {
        nusb::ErrorKind::PermissionDenied => LogicError::Permission(permission_help().into()),
        nusb::ErrorKind::Busy => LogicError::Usb("the board is in use by another program (PulseView, sigrok-cli, Saleae Logic); close it and retry".into()),
        nusb::ErrorKind::Disconnected | nusb::ErrorKind::NotFound => LogicError::NoDevice,
        _ => usb_err("cannot open the board", e),
    }
}

impl UsbBackend for NusbBackend {
    fn list(&self) -> Result<Vec<UsbDeviceInfo>> {
        let devs = nusb::list_devices().wait().map_err(|e| usb_err("cannot list USB devices", e))?;
        Ok(devs.filter(|d| board_for(d.vendor_id(), d.product_id()).is_some()).map(|d| info_of(&d)).collect())
    }

    fn open(&self, id: &str) -> Result<Box<dyn UsbLink>> {
        let devs = nusb::list_devices().wait().map_err(|e| usb_err("cannot list USB devices", e))?;
        let d = devs.into_iter().find(|d| info_of(d).id == id).ok_or(LogicError::NoDevice)?;
        let dev = d.open().wait().map_err(open_err)?;
        let iface = dev.detach_and_claim_interface(0).wait().map_err(open_err)?;
        Ok(Box::new(NusbLink { iface, ep: None }))
    }
}

struct NusbLink {
    iface: nusb::Interface,
    ep: Option<nusb::Endpoint<Bulk, In>>,
}

impl UsbLink for NusbLink {
    fn control_out(&mut self, request: u8, value: u16, index: u16, data: &[u8]) -> Result<()> {
        self.iface
            .control_out(ControlOut { control_type: ControlType::Vendor, recipient: Recipient::Device, request, value, index, data }, TIMEOUT)
            .wait()
            .map_err(|e| usb_err(&format!("request 0x{request:02X} failed"), e))
    }

    fn control_in(&mut self, request: u8, value: u16, index: u16, len: u16) -> Result<Vec<u8>> {
        self.iface
            .control_in(ControlIn { control_type: ControlType::Vendor, recipient: Recipient::Device, request, value, index, length: len }, TIMEOUT)
            .wait()
            .map_err(|e| usb_err(&format!("request 0x{request:02X} failed"), e))
    }

    fn start_bulk(&mut self, ep: u8, size: usize, transfers: usize) -> Result<()> {
        let mut e = self.iface.endpoint::<Bulk, In>(ep).map_err(|e| usb_err("bulk endpoint", e))?;
        for _ in 0..transfers {
            e.submit(Buffer::new(size));
        }
        self.ep = Some(e);
        Ok(())
    }

    fn read_bulk(&mut self, timeout: Duration) -> Result<Option<Vec<u8>>> {
        let ep = self.ep.as_mut().ok_or_else(|| LogicError::Usb("acquisition not started".into()))?;
        let Some(c) = ep.wait_next_complete(timeout) else { return Ok(None) };
        match c.status {
            Ok(()) => {
                let data = c.buffer[..c.actual_len].to_vec();
                let mut buf = c.buffer;
                buf.clear();
                let size = buf.capacity();
                buf.set_requested_len(size);
                ep.submit(buf);
                Ok(Some(data))
            }
            Err(nusb::transfer::TransferError::Disconnected) => Err(LogicError::Usb("the board was unplugged".into())),
            Err(e) => Err(usb_err("USB transfer failed", e)),
        }
    }
}

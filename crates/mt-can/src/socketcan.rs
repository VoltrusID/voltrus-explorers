//! Linux SocketCAN (raw socket, CAN FD frames enabled, error frames reported). The interface must
//! already be up, e.g. `ip link set can0 up type can bitrate 500000`.

use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};

use tokio::io::Interest;
use tokio::io::unix::AsyncFd;

use crate::bus::{BusEvent, CanHandle};
use crate::error::CanError;
use crate::frame::{CanFrame, Direction, TimedFrame, now_us};

const AF_CAN: libc::c_int = 29;
const CAN_RAW: libc::c_int = 1;
const SOL_CAN_RAW: libc::c_int = 101;
const CAN_RAW_ERR_FILTER: libc::c_int = 2;
const CAN_RAW_FD_FRAMES: libc::c_int = 5;
const CAN_EFF_FLAG: u32 = 0x8000_0000;
const CAN_RTR_FLAG: u32 = 0x4000_0000;
const CAN_ERR_FLAG: u32 = 0x2000_0000;
const CAN_EFF_MASK: u32 = 0x1FFF_FFFF;
const CAN_SFF_MASK: u32 = 0x7FF;
const CANFD_BRS: u8 = 0x01;
const CANFD_ESI: u8 = 0x02;
const CANFD_FDF: u8 = 0x04;
const CAN_MTU: usize = 16;
const CANFD_MTU: usize = 72;

#[repr(C)]
struct SockaddrCan {
    can_family: libc::sa_family_t,
    can_ifindex: libc::c_int,
    // The tp / j1939 union: 16 bytes, 8-byte aligned.
    addr: [u64; 2],
}

#[repr(C, align(8))]
struct CanFdFrame {
    can_id: u32,
    len: u8,
    flags: u8,
    res0: u8,
    res1: u8,
    data: [u8; 64],
}

fn os_err(what: &str) -> CanError {
    CanError::Adapter(format!("{what}: {}", std::io::Error::last_os_error()))
}

pub(crate) fn open(ifname: &str, description: String) -> Result<CanHandle, CanError> {
    let cname = std::ffi::CString::new(ifname).map_err(|_| CanError::Adapter("bad interface name".into()))?;
    // SAFETY: plain libc calls on a socket this function owns; structs are repr(C) and sized as the kernel expects.
    let fd = unsafe {
        let index = libc::if_nametoindex(cname.as_ptr());
        if index == 0 {
            return Err(CanError::Adapter(format!("no network interface {ifname} (is the CAN driver loaded and the link up?)")));
        }
        let raw = libc::socket(AF_CAN, libc::SOCK_RAW | libc::SOCK_NONBLOCK | libc::SOCK_CLOEXEC, CAN_RAW);
        if raw < 0 {
            return Err(os_err("cannot create a CAN socket"));
        }
        let fd = OwnedFd::from_raw_fd(raw);
        let on: libc::c_int = 1;
        // Older kernels and some drivers have no CAN FD; classic frames still work.
        libc::setsockopt(
            raw,
            SOL_CAN_RAW,
            CAN_RAW_FD_FRAMES,
            &on as *const _ as *const libc::c_void,
            std::mem::size_of::<libc::c_int>() as libc::socklen_t,
        );
        let err_mask: u32 = 0x1FF;
        libc::setsockopt(
            raw,
            SOL_CAN_RAW,
            CAN_RAW_ERR_FILTER,
            &err_mask as *const _ as *const libc::c_void,
            std::mem::size_of::<u32>() as libc::socklen_t,
        );
        let addr = SockaddrCan { can_family: AF_CAN as libc::sa_family_t, can_ifindex: index as libc::c_int, addr: [0; 2] };
        if libc::bind(raw, &addr as *const _ as *const libc::sockaddr, std::mem::size_of::<SockaddrCan>() as libc::socklen_t) < 0 {
            return Err(os_err(&format!("cannot bind to {ifname}")));
        }
        fd
    };
    let afd = AsyncFd::with_interest(fd, Interest::READABLE | Interest::WRITABLE).map_err(CanError::Io)?;
    let (mut d, make) = crate::bus::channels(description);
    let task = tokio::spawn(async move {
        let reason = loop {
            tokio::select! {
                _ = &mut d.stop => break None,
                r = afd.readable() => {
                    let mut guard = match r { Ok(g) => g, Err(e) => break Some(e.to_string()) };
                    let mut frame = CanFdFrame { can_id: 0, len: 0, flags: 0, res0: 0, res1: 0, data: [0; 64] };
                    let res = guard.try_io(|inner| {
                        // SAFETY: reading into a 72-byte repr(C) buffer from our own socket.
                        let n = unsafe { libc::read(inner.as_raw_fd(), &mut frame as *mut _ as *mut libc::c_void, CANFD_MTU) };
                        if n < 0 { Err(std::io::Error::last_os_error()) } else { Ok(n as usize) }
                    });
                    match res {
                        Err(_would_block) => continue,
                        Ok(Err(e)) => break Some(format!("read from the CAN socket failed: {e}")),
                        Ok(Ok(n)) => {
                            if let Some(ev) = from_kernel(&frame, n) {
                                let _ = d.events.send(ev);
                            }
                        }
                    }
                }
                req = d.requests.recv() => match req {
                    Some((frame, done)) => {
                        let r = write_frame(&afd, &frame).await;
                        if r.is_ok() {
                            let _ = d.events.send(BusEvent::Frame(TimedFrame { ts_us: now_us(), dir: Direction::Tx, frame }));
                        }
                        let _ = done.send(r);
                    }
                    None => break None,
                },
            }
        };
        let _ = d.events.send(BusEvent::Closed(reason));
    });
    Ok(make(task))
}

fn from_kernel(f: &CanFdFrame, n: usize) -> Option<BusEvent> {
    if f.can_id & CAN_ERR_FLAG != 0 {
        return Some(BusEvent::Error(error_text(f.can_id & CAN_EFF_MASK)));
    }
    let extended = f.can_id & CAN_EFF_FLAG != 0;
    let id = if extended { f.can_id & CAN_EFF_MASK } else { f.can_id & CAN_SFF_MASK };
    let frame = if n == CANFD_MTU {
        let len = (f.len as usize).min(64);
        CanFrame { id, extended, rtr: false, fd: true, brs: f.flags & CANFD_BRS != 0, esi: f.flags & CANFD_ESI != 0, data: f.data[..len].to_vec(), rtr_len: 0 }
    } else if n == CAN_MTU {
        let len = (f.len as usize).min(8);
        if f.can_id & CAN_RTR_FLAG != 0 {
            CanFrame { id, extended, rtr: true, fd: false, brs: false, esi: false, data: Vec::new(), rtr_len: len as u8 }
        } else {
            CanFrame { id, extended, rtr: false, fd: false, brs: false, esi: false, data: f.data[..len].to_vec(), rtr_len: 0 }
        }
    } else {
        return Some(BusEvent::Error(format!("short read from the CAN socket ({n} bytes)")));
    };
    Some(BusEvent::Frame(TimedFrame { ts_us: now_us(), dir: Direction::Rx, frame }))
}

async fn write_frame(afd: &AsyncFd<OwnedFd>, frame: &CanFrame) -> Result<(), CanError> {
    let mut k = CanFdFrame { can_id: frame.id, len: 0, flags: 0, res0: 0, res1: 0, data: [0; 64] };
    if frame.extended {
        k.can_id |= CAN_EFF_FLAG;
    }
    let size = if frame.fd {
        k.len = frame.data.len() as u8;
        k.flags = CANFD_FDF | if frame.brs { CANFD_BRS } else { 0 };
        k.data[..frame.data.len()].copy_from_slice(&frame.data);
        CANFD_MTU
    } else if frame.rtr {
        k.can_id |= CAN_RTR_FLAG;
        k.len = frame.rtr_len;
        CAN_MTU
    } else {
        k.len = frame.data.len() as u8;
        k.data[..frame.data.len()].copy_from_slice(&frame.data);
        CAN_MTU
    };
    loop {
        let mut guard = afd.writable().await?;
        let r = guard.try_io(|inner| {
            // SAFETY: writing `size` bytes of a 72-byte repr(C) struct to our own socket.
            let n = unsafe { libc::write(inner.as_raw_fd(), &k as *const _ as *const libc::c_void, size) };
            if n < 0 { Err(std::io::Error::last_os_error()) } else { Ok(n as usize) }
        });
        match r {
            Err(_would_block) => continue,
            Ok(Ok(_)) => return Ok(()),
            Ok(Err(e)) if e.raw_os_error() == Some(libc::ENOBUFS) => {
                return Err(CanError::Adapter("transmit queue full (is anything acknowledging on the bus?)".into()));
            }
            Ok(Err(e)) if e.raw_os_error() == Some(libc::EINVAL) && frame.fd => {
                return Err(CanError::Adapter("this interface does not do CAN FD".into()));
            }
            Ok(Err(e)) => return Err(e.into()),
        }
    }
}

fn error_text(class: u32) -> String {
    const NAMES: [&str; 9] = [
        "transmit timeout",
        "arbitration lost",
        "controller problem",
        "protocol violation",
        "transceiver status",
        "no acknowledge",
        "bus off",
        "bus error",
        "controller restarted",
    ];
    let parts: Vec<&str> = (0..9).filter(|b| class & (1 << b) != 0).map(|b| NAMES[b]).collect();
    if parts.is_empty() { "error frame".into() } else { format!("error frame: {}", parts.join(", ")) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layouts_match_the_kernel() {
        assert_eq!(std::mem::size_of::<SockaddrCan>(), 24);
        assert_eq!(std::mem::size_of::<CanFdFrame>(), CANFD_MTU);
    }

    #[test]
    fn kernel_frames() {
        let mut k = CanFdFrame { can_id: 0x123, len: 2, flags: 0, res0: 0, res1: 0, data: [0; 64] };
        k.data[0] = 0xAB;
        let Some(BusEvent::Frame(f)) = from_kernel(&k, CAN_MTU) else { panic!() };
        assert_eq!(f.frame, CanFrame::new(0x123, &[0xAB, 0]).unwrap());
        k.can_id = 0x1234 | CAN_EFF_FLAG | CAN_RTR_FLAG;
        let Some(BusEvent::Frame(f)) = from_kernel(&k, CAN_MTU) else { panic!() };
        assert!(f.frame.rtr && f.frame.extended && f.frame.rtr_len == 2);
        let mut fd = CanFdFrame { can_id: 0x201, len: 64, flags: CANFD_FDF | CANFD_BRS, res0: 0, res1: 0, data: [0x5A; 64] };
        let Some(BusEvent::Frame(f)) = from_kernel(&fd, CANFD_MTU) else { panic!() };
        assert!(f.frame.fd && f.frame.brs && !f.frame.esi && f.frame.data.len() == 64 && f.frame.dlc() == 15);
        fd.can_id = 0x1234_5678 | CAN_EFF_FLAG;
        fd.len = 20;
        fd.flags = CANFD_FDF | CANFD_ESI;
        let Some(BusEvent::Frame(f)) = from_kernel(&fd, CANFD_MTU) else { panic!() };
        assert!(f.frame.extended && f.frame.id == 0x1234_5678 && !f.frame.brs && f.frame.esi && f.frame.data.len() == 20);
        assert!(matches!(from_kernel(&fd, 40), Some(BusEvent::Error(_))));
        k.can_id = CAN_ERR_FLAG | 0x40;
        assert_eq!(from_kernel(&k, CAN_MTU), Some(BusEvent::Error("error frame: bus off".into())));
    }
}

//! Linux SocketCAN (raw socket, CAN FD frames enabled, error frames reported). The interface must
//! already be up, e.g. `ip link set can0 up type can bitrate 500000`.
//!
//! Received frames carry the kernel's receive timestamp (`SO_TIMESTAMP`), not the time this
//! process got round to reading them, and frames the kernel had to drop because the socket buffer
//! was full are counted (`SO_RXQ_OVFL`) and reported.

use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};

use tokio::io::Interest;
use tokio::io::unix::AsyncFd;

use crate::bus::{BusEvent, CanHandle};
use crate::error::CanError;
use crate::frame::{BusStatus, CanFrame, Direction, TimedFrame, decode_error_frame, now_us};

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
/// Socket receive buffer asked for: about 4 MB holds several seconds of a saturated 1 Mbit/s bus
/// when the app stalls. The kernel caps it at net.core.rmem_max; smaller sizes are tried next.
const RCVBUF_SIZES: [libc::c_int; 3] = [4 << 20, 1 << 20, 256 << 10];

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

impl CanFdFrame {
    fn zeroed() -> Self {
        CanFdFrame { can_id: 0, len: 0, flags: 0, res0: 0, res1: 0, data: [0; 64] }
    }
}

fn os_err(what: &str) -> CanError {
    CanError::Adapter(format!("{what}: {}", std::io::Error::last_os_error()))
}

fn set_int(fd: libc::c_int, level: libc::c_int, name: libc::c_int, v: libc::c_int) -> bool {
    // SAFETY: setsockopt with a pointer to a live c_int and its size.
    unsafe {
        libc::setsockopt(fd, level, name, &v as *const _ as *const libc::c_void, std::mem::size_of::<libc::c_int>() as libc::socklen_t) == 0
    }
}

/// A non-blocking raw CAN socket bound to `ifname`, with FD frames, error frames, a large receive
/// buffer, drop counting and receive timestamps. Returns the socket and whether it carries CAN FD.
fn open_socket(ifname: &str) -> Result<(OwnedFd, bool), CanError> {
    let cname = std::ffi::CString::new(ifname).map_err(|_| CanError::Adapter("bad interface name".into()))?;
    // SAFETY: plain libc calls on a socket this function owns; structs are repr(C) and sized as the kernel expects.
    let (fd, fd_frames) = unsafe {
        let index = libc::if_nametoindex(cname.as_ptr());
        if index == 0 {
            return Err(CanError::Adapter(format!("no network interface {ifname} (is the CAN driver loaded and the link up?)")));
        }
        let raw = libc::socket(AF_CAN, libc::SOCK_RAW | libc::SOCK_NONBLOCK | libc::SOCK_CLOEXEC, CAN_RAW);
        if raw < 0 {
            return Err(os_err("cannot create a CAN socket"));
        }
        let fd = OwnedFd::from_raw_fd(raw);
        // Older kernels and some drivers have no CAN FD; classic frames still work.
        let fd_frames = set_int(raw, SOL_CAN_RAW, CAN_RAW_FD_FRAMES, 1);
        let err_mask: u32 = 0x3FF;
        libc::setsockopt(
            raw,
            SOL_CAN_RAW,
            CAN_RAW_ERR_FILTER,
            &err_mask as *const _ as *const libc::c_void,
            std::mem::size_of::<u32>() as libc::socklen_t,
        );
        // SO_RCVBUFFORCE passes rmem_max with CAP_NET_ADMIN; otherwise the largest size allowed.
        if !RCVBUF_SIZES.iter().any(|&s| set_int(raw, libc::SOL_SOCKET, libc::SO_RCVBUFFORCE, s)) {
            for s in RCVBUF_SIZES {
                if set_int(raw, libc::SOL_SOCKET, libc::SO_RCVBUF, s) {
                    break;
                }
            }
        }
        set_int(raw, libc::SOL_SOCKET, libc::SO_RXQ_OVFL, 1);
        set_int(raw, libc::SOL_SOCKET, libc::SO_TIMESTAMP, 1);
        let addr = SockaddrCan { can_family: AF_CAN as libc::sa_family_t, can_ifindex: index as libc::c_int, addr: [0; 2] };
        if libc::bind(raw, &addr as *const _ as *const libc::sockaddr, std::mem::size_of::<SockaddrCan>() as libc::socklen_t) < 0 {
            return Err(os_err(&format!("cannot bind to {ifname}")));
        }
        (fd, fd_frames)
    };
    // A CAN FD capable interface has an MTU of 72 (CAN XL ones more); a classic one 16.
    let mtu: usize = std::fs::read_to_string(format!("/sys/class/net/{ifname}/mtu")).ok().and_then(|s| s.trim().parse().ok()).unwrap_or(CAN_MTU);
    Ok((fd, fd_frames && mtu >= CANFD_MTU))
}

/// One read from the socket.
struct Received {
    frame: CanFdFrame,
    len: usize,
    /// Kernel receive time, microseconds since the Unix epoch.
    ts_us: Option<u64>,
    /// Frames the kernel dropped for this socket so far.
    dropped: Option<u32>,
}

fn recv_frame(fd: libc::c_int) -> std::io::Result<Received> {
    let mut frame = CanFdFrame::zeroed();
    let mut iov = libc::iovec { iov_base: &mut frame as *mut _ as *mut libc::c_void, iov_len: CANFD_MTU };
    // Room for a timeval and a u32 with their headers, 8-byte aligned.
    let mut control = [0u64; 16];
    // SAFETY: msghdr is plain data; zeroed is a valid empty header, filled in below.
    let mut msg: libc::msghdr = unsafe { std::mem::zeroed() };
    msg.msg_iov = &mut iov;
    msg.msg_iovlen = 1;
    msg.msg_control = control.as_mut_ptr() as *mut libc::c_void;
    msg.msg_controllen = std::mem::size_of_val(&control) as _;
    // SAFETY: recvmsg into buffers that live for the call and are sized in `msg`.
    let n = unsafe { libc::recvmsg(fd, &mut msg, 0) };
    if n < 0 {
        return Err(std::io::Error::last_os_error());
    }
    let (mut ts_us, mut dropped) = (None, None);
    // SAFETY: walking the control messages the kernel wrote, with the libc CMSG helpers; the
    // payloads are read unaligned.
    unsafe {
        let mut c = libc::CMSG_FIRSTHDR(&msg);
        while !c.is_null() {
            let h = &*c;
            if h.cmsg_level == libc::SOL_SOCKET && h.cmsg_type == libc::SCM_TIMESTAMP {
                let tv = std::ptr::read_unaligned(libc::CMSG_DATA(c) as *const libc::timeval);
                ts_us = Some(tv.tv_sec as u64 * 1_000_000 + tv.tv_usec as u64);
            } else if h.cmsg_level == libc::SOL_SOCKET && h.cmsg_type == libc::SO_RXQ_OVFL {
                dropped = Some(std::ptr::read_unaligned(libc::CMSG_DATA(c) as *const u32));
            }
            c = libc::CMSG_NXTHDR(&msg, c);
        }
    }
    Ok(Received { frame, len: n as usize, ts_us, dropped })
}

pub(crate) fn open(ifname: &str, description: String) -> Result<CanHandle, CanError> {
    let (fd, fd_frames) = open_socket(ifname)?;
    let afd = AsyncFd::with_interest(fd, Interest::READABLE | Interest::WRITABLE).map_err(CanError::Io)?;
    let (mut d, make) = crate::bus::channels(description, fd_frames);
    if !fd_frames {
        let _ = d.events.send(BusEvent::Info(format!("{ifname} is classic CAN (MTU 16): CAN FD frames cannot be sent or received")));
    }
    let task = tokio::spawn(async move {
        let mut status = BusStatus::default();
        let mut dropped_seen = 0u32;
        let reason = loop {
            tokio::select! {
                _ = &mut d.stop => break None,
                r = afd.readable() => {
                    let mut guard = match r { Ok(g) => g, Err(e) => break Some(e.to_string()) };
                    match guard.try_io(|inner| recv_frame(inner.as_raw_fd())) {
                        Err(_would_block) => continue,
                        Ok(Err(e)) => break Some(format!("read from the CAN socket failed: {e}")),
                        Ok(Ok(r)) => {
                            if let Some(total) = r.dropped
                                && total != dropped_seen
                            {
                                let lost = total.wrapping_sub(dropped_seen);
                                dropped_seen = total;
                                let _ = d.events.send(BusEvent::Error(format!(
                                    "the kernel dropped {lost} received frame(s): the socket buffer was full ({total} lost since connecting)"
                                )));
                            }
                            match from_kernel(&r.frame, r.len, r.ts_us.unwrap_or_else(now_us)) {
                                Some(Kernel::Frame(f)) => { let _ = d.events.send(BusEvent::Frame(f)); }
                                Some(Kernel::Error(class, data)) => {
                                    let e = decode_error_frame(class, &data);
                                    if status.update(&e) {
                                        let _ = d.events.send(BusEvent::Status(status));
                                    }
                                    let _ = d.events.send(BusEvent::Error(e.text));
                                }
                                Some(Kernel::Bad(m)) => { let _ = d.events.send(BusEvent::Error(m)); }
                                None => {}
                            }
                        }
                    }
                }
                req = d.requests.recv() => match req {
                    Some((frame, done)) => {
                        let r = write_frame(&afd, &frame).await;
                        if r.is_ok() {
                            // Queued in the kernel; the driver sends it when the bus allows.
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

enum Kernel {
    Frame(TimedFrame),
    /// Error frame: class bits and data.
    Error(u32, [u8; 8]),
    Bad(String),
}

fn from_kernel(f: &CanFdFrame, n: usize, ts_us: u64) -> Option<Kernel> {
    if f.can_id & CAN_ERR_FLAG != 0 {
        let mut data = [0u8; 8];
        data.copy_from_slice(&f.data[..8]);
        return Some(Kernel::Error(f.can_id & CAN_EFF_MASK, data));
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
        return Some(Kernel::Bad(format!("short read from the CAN socket ({n} bytes)")));
    };
    Some(Kernel::Frame(TimedFrame { ts_us, dir: Direction::Rx, frame }))
}

async fn write_frame(afd: &AsyncFd<OwnedFd>, frame: &CanFrame) -> Result<(), CanError> {
    let mut k = CanFdFrame { can_id: frame.id, ..CanFdFrame::zeroed() };
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
                return Err(CanError::Adapter(format!("transmit queue full: {}", crate::frame::NO_ACK_HINT)));
            }
            Ok(Err(e)) if e.raw_os_error() == Some(libc::EINVAL) && frame.fd => {
                return Err(CanError::Adapter("this interface does not do CAN FD".into()));
            }
            Ok(Err(e)) => return Err(e.into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layouts_match_the_kernel() {
        assert_eq!(std::mem::size_of::<SockaddrCan>(), 24);
        assert_eq!(std::mem::size_of::<CanFdFrame>(), CANFD_MTU);
    }

    fn frame_of(k: Option<Kernel>) -> TimedFrame {
        match k {
            Some(Kernel::Frame(f)) => f,
            _ => panic!("not a frame"),
        }
    }

    #[test]
    fn kernel_frames() {
        let mut k = CanFdFrame { can_id: 0x123, len: 2, ..CanFdFrame::zeroed() };
        k.data[0] = 0xAB;
        let f = frame_of(from_kernel(&k, CAN_MTU, 42));
        assert_eq!((f.frame, f.ts_us), (CanFrame::new(0x123, &[0xAB, 0]).unwrap(), 42));
        k.can_id = 0x1234 | CAN_EFF_FLAG | CAN_RTR_FLAG;
        let f = frame_of(from_kernel(&k, CAN_MTU, 0));
        assert!(f.frame.rtr && f.frame.extended && f.frame.rtr_len == 2);
        let mut fd = CanFdFrame { can_id: 0x201, len: 64, flags: CANFD_FDF | CANFD_BRS, res0: 0, res1: 0, data: [0x5A; 64] };
        let f = frame_of(from_kernel(&fd, CANFD_MTU, 0));
        assert!(f.frame.fd && f.frame.brs && !f.frame.esi && f.frame.data.len() == 64 && f.frame.dlc() == 15);
        fd.can_id = 0x1234_5678 | CAN_EFF_FLAG;
        fd.len = 20;
        fd.flags = CANFD_FDF | CANFD_ESI;
        let f = frame_of(from_kernel(&fd, CANFD_MTU, 0));
        assert!(f.frame.extended && f.frame.id == 0x1234_5678 && !f.frame.brs && f.frame.esi && f.frame.data.len() == 20);
        assert!(matches!(from_kernel(&fd, 40, 0), Some(Kernel::Bad(_))));
        // An error frame as a Linux driver (e.g. mcp251xfd) sends it: controller TX passive, counters.
        let mut e = CanFdFrame { can_id: CAN_ERR_FLAG | 0x004 | 0x200, len: 8, ..CanFdFrame::zeroed() };
        e.data[1] = 0x20;
        e.data[6] = 128;
        let Some(Kernel::Error(class, data)) = from_kernel(&e, CAN_MTU, 0) else { panic!() };
        let d = decode_error_frame(class, &data);
        assert_eq!(d.state, Some(crate::frame::BusState::ErrorPassive));
        assert_eq!(d.counters, Some((128, 0)));
        k.can_id = CAN_ERR_FLAG | 0x40;
        let Some(Kernel::Error(class, data)) = from_kernel(&k, CAN_MTU, 0) else { panic!() };
        assert!(decode_error_frame(class, &data).text.starts_with("error frame: bus off"));
    }

    /// Against the host's vcan0 when it exists and is up (skipped otherwise).
    fn vcan0() -> Option<&'static str> {
        let flags = std::fs::read_to_string("/sys/class/net/vcan0/flags").ok()?;
        (u32::from_str_radix(flags.trim().trim_start_matches("0x"), 16).ok()? & 1 != 0).then_some("vcan0")
    }

    fn get_int(fd: &OwnedFd, name: libc::c_int) -> libc::c_int {
        let mut v: libc::c_int = 0;
        let mut len = std::mem::size_of::<libc::c_int>() as libc::socklen_t;
        // SAFETY: getsockopt into a live c_int.
        unsafe { libc::getsockopt(fd.as_raw_fd(), libc::SOL_SOCKET, name, &mut v as *mut _ as *mut libc::c_void, &mut len) };
        v
    }

    fn send_classic(fd: &OwnedFd, id: u32, b: u8) -> bool {
        let mut k = CanFdFrame { can_id: id, len: 1, ..CanFdFrame::zeroed() };
        k.data[0] = b;
        // SAFETY: writes 16 bytes of a 72-byte repr(C) struct.
        unsafe { libc::write(fd.as_raw_fd(), &k as *const _ as *const libc::c_void, CAN_MTU) == CAN_MTU as isize }
    }

    #[test]
    fn vcan0_buffer_timestamps_and_drops() {
        let Some(ifname) = vcan0() else {
            eprintln!("skipped: no vcan0 that is up");
            return;
        };
        let (rx, fd) = open_socket(ifname).unwrap();
        let mtu: usize = std::fs::read_to_string("/sys/class/net/vcan0/mtu").unwrap().trim().parse().unwrap();
        assert_eq!(fd, mtu >= CANFD_MTU);
        let rmem_max: i32 = std::fs::read_to_string("/proc/sys/net/core/rmem_max").unwrap().trim().parse().unwrap();
        let rcvbuf = get_int(&rx, libc::SO_RCVBUF);
        eprintln!("SO_RCVBUF {rcvbuf} (rmem_max {rmem_max})");
        // The kernel doubles the size asked for; it got at least the largest size rmem_max allows.
        let allowed = RCVBUF_SIZES.iter().copied().find(|s| *s <= rmem_max).unwrap_or(0);
        assert!(rcvbuf >= allowed, "SO_RCVBUF {rcvbuf} < {allowed}");
        assert_eq!(get_int(&rx, libc::SO_RXQ_OVFL), 1);

        // Kernel timestamps: the time the frame arrived, not the time it was read.
        let (tx, _) = open_socket(ifname).unwrap();
        let before = now_us();
        assert!(send_classic(&tx, 0x5B1, 1));
        std::thread::sleep(std::time::Duration::from_millis(80));
        let r = loop {
            let r = recv_frame(rx.as_raw_fd()).unwrap();
            if r.frame.can_id == 0x5B1 {
                break r;
            }
        };
        let after = now_us();
        let ts = r.ts_us.expect("SO_TIMESTAMP");
        assert!(ts >= before && ts + 60_000 < after, "ts {ts} not between {before} and {after} - 60 ms");
        // The kernel only attaches the drop count once something was dropped.
        assert_eq!(r.dropped.unwrap_or(0), 0);
        drop(rx);

        // Drops: a small buffer that nobody reads, flooded, then read: SO_RXQ_OVFL counts the loss.
        let (small, _) = open_socket(ifname).unwrap();
        assert!(set_int(small.as_raw_fd(), libc::SOL_SOCKET, libc::SO_RCVBUF, 4096));
        let mut sent = 0;
        for i in 0..2000u32 {
            if send_classic(&tx, 0x5B2, i as u8) {
                sent += 1;
            }
        }
        assert!(sent > 500, "only {sent} frames written");
        // The count rides on frames queued after the loss, so drain, then let one more in.
        while recv_frame(small.as_raw_fd()).is_ok() {}
        assert!(send_classic(&tx, 0x5B3, 0));
        let mut dropped = 0;
        while let Ok(r) = recv_frame(small.as_raw_fd()) {
            dropped = dropped.max(r.dropped.unwrap_or(0));
        }
        assert!(dropped > 0, "no drops counted after {sent} frames into a 4 kB buffer");
        eprintln!("{sent} frames sent, {dropped} dropped and counted");
    }
}

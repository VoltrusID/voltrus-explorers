//! CAN FD on a real SocketCAN interface: the driver against an independent raw socket on `vcan0`.
//! Skipped (passes with a note) when the host has no `vcan0`, or it is down or not FD capable. Create one with
//! `sudo ip link add vcan0 type vcan && sudo ip link set vcan0 mtu 72 up`.
#![cfg(target_os = "linux")]

use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::time::Duration;

use mt_can::{BusEvent, CanFrame, Direction, InterfaceConfig, VirtualBus};

const IFACE: &str = "vcan0";
const AF_CAN: libc::c_int = 29;
const CAN_RAW: libc::c_int = 1;
const SOL_CAN_RAW: libc::c_int = 101;
const CAN_RAW_FD_FRAMES: libc::c_int = 5;
const CAN_EFF_FLAG: u32 = 0x8000_0000;
const CANFD_BRS: u8 = 0x01;
const CANFD_ESI: u8 = 0x02;
const CANFD_FDF: u8 = 0x04;

// Identifiers nothing else on the user's vcan0 uses (the yocto bridge sends 0x100/0x101/0x200/0x201).
const CLASSIC_ID: u32 = 0x5A1;
const FD_ID: u32 = 0x5A2;
const FD_EXT_ID: u32 = 0x1ABC_DE5A;
const REPLY_ID: u32 = 0x5A3;

#[repr(C)]
struct SockaddrCan {
    can_family: libc::sa_family_t,
    can_ifindex: libc::c_int,
    addr: [u64; 2],
}

#[repr(C, align(8))]
struct KFrame {
    can_id: u32,
    len: u8,
    flags: u8,
    res: [u8; 2],
    data: [u8; 64],
}

/// The interface's MTU when it exists and is up, from sysfs.
fn iface_mtu() -> Option<u32> {
    let base = format!("/sys/class/net/{IFACE}");
    let flags = std::fs::read_to_string(format!("{base}/flags")).ok()?;
    let up = u32::from_str_radix(flags.trim().trim_start_matches("0x"), 16).ok()? & 1 != 0;
    if !up {
        return None;
    }
    std::fs::read_to_string(format!("{base}/mtu")).ok()?.trim().parse().ok()
}

/// A plain blocking raw CAN socket with FD frames on, bound to the interface.
fn raw_socket() -> OwnedFd {
    let name = std::ffi::CString::new(IFACE).unwrap();
    // SAFETY: libc calls on a socket owned here; the structs match the kernel's layouts.
    unsafe {
        let index = libc::if_nametoindex(name.as_ptr());
        assert!(index != 0);
        let raw = libc::socket(AF_CAN, libc::SOCK_RAW | libc::SOCK_CLOEXEC, CAN_RAW);
        assert!(raw >= 0, "socket: {}", std::io::Error::last_os_error());
        let fd = OwnedFd::from_raw_fd(raw);
        let on: libc::c_int = 1;
        let r = libc::setsockopt(raw, SOL_CAN_RAW, CAN_RAW_FD_FRAMES, &on as *const _ as *const libc::c_void, 4);
        assert_eq!(r, 0, "CAN_RAW_FD_FRAMES: {}", std::io::Error::last_os_error());
        let tv = libc::timeval { tv_sec: 2, tv_usec: 0 };
        libc::setsockopt(
            raw,
            libc::SOL_SOCKET,
            libc::SO_RCVTIMEO,
            &tv as *const _ as *const libc::c_void,
            std::mem::size_of::<libc::timeval>() as libc::socklen_t,
        );
        let addr = SockaddrCan { can_family: AF_CAN as libc::sa_family_t, can_ifindex: index as libc::c_int, addr: [0; 2] };
        let r = libc::bind(raw, &addr as *const _ as *const libc::sockaddr, std::mem::size_of::<SockaddrCan>() as libc::socklen_t);
        assert_eq!(r, 0, "bind: {}", std::io::Error::last_os_error());
        fd
    }
}

fn raw_write(fd: &OwnedFd, can_id: u32, flags: u8, data: &[u8], fd_frame: bool) {
    let mut k = KFrame { can_id, len: data.len() as u8, flags, res: [0; 2], data: [0; 64] };
    k.data[..data.len()].copy_from_slice(data);
    let size = if fd_frame { 72 } else { 16 };
    // SAFETY: writes `size` bytes of a 72-byte repr(C) struct.
    let n = unsafe { libc::write(fd.as_raw_fd(), &k as *const _ as *const libc::c_void, size) };
    assert_eq!(n, size as isize, "write: {}", std::io::Error::last_os_error());
}

/// Reads frames until one has `can_id`; returns it with the byte count of the read.
fn raw_read_id(fd: &OwnedFd, can_id: u32) -> (KFrame, usize) {
    loop {
        let mut k = KFrame { can_id: 0, len: 0, flags: 0, res: [0; 2], data: [0; 64] };
        // SAFETY: reads at most 72 bytes into a 72-byte repr(C) struct.
        let n = unsafe { libc::read(fd.as_raw_fd(), &mut k as *mut _ as *mut libc::c_void, 72) };
        assert!(n > 0, "no frame {can_id:X} on {IFACE}: {}", std::io::Error::last_os_error());
        if k.can_id == can_id {
            return (k, n as usize);
        }
    }
}

async fn next_frame(ev: &mut tokio::sync::mpsc::UnboundedReceiver<BusEvent>, id: u32) -> mt_can::TimedFrame {
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            match ev.recv().await {
                Some(BusEvent::Frame(f)) if f.frame.id == id && f.dir == Direction::Rx => return f,
                Some(_) => continue,
                None => panic!("the bus closed"),
            }
        }
    })
    .await
    .unwrap_or_else(|_| panic!("frame {id:X} did not arrive"))
}

#[tokio::test]
async fn socketcan_fd_on_vcan0() {
    match iface_mtu() {
        Some(72..) => {}
        Some(mtu) => {
            eprintln!("skipped: {IFACE} has MTU {mtu}, not CAN FD capable (needs 72)");
            return;
        }
        None => {
            eprintln!("skipped: no {IFACE} that is up");
            return;
        }
    }
    let cfg = InterfaceConfig::Socketcan { ifname: IFACE.into() };
    let mut h = mt_can::open(&cfg, &VirtualBus::new()).await.expect("open vcan0");
    let mut ev = h.take_events().unwrap();
    let peer = raw_socket();

    // Classic frame in.
    raw_write(&peer, CLASSIC_ID, 0, &[0x48, 0x42, 0x54, 0x31, 1, 2, 3, 4], false);
    let c = next_frame(&mut ev, CLASSIC_ID).await.frame;
    assert_eq!(c, CanFrame::new(CLASSIC_ID, &[0x48, 0x42, 0x54, 0x31, 1, 2, 3, 4]).unwrap());

    // 64-byte FD frame with bit rate switch in, then a 29-bit one with ESI and a 12-byte payload.
    let payload: Vec<u8> = (0..64).collect();
    raw_write(&peer, FD_ID, CANFD_FDF | CANFD_BRS, &payload, true);
    let f = next_frame(&mut ev, FD_ID).await.frame;
    assert!(f.fd && f.brs && !f.esi && !f.extended);
    assert_eq!(f.data, payload);
    assert_eq!(f.dlc(), 15);
    raw_write(&peer, FD_EXT_ID | CAN_EFF_FLAG, CANFD_FDF | CANFD_ESI, &payload[..12], true);
    let e = next_frame(&mut ev, FD_EXT_ID).await.frame;
    assert!(e.fd && !e.brs && e.esi && e.extended);
    assert_eq!(e.data, &payload[..12]);
    assert_eq!(e.dlc(), 9);

    // FD and classic out, read back by the peer socket.
    let out: Vec<u8> = (0..48).map(|i| 0xFF - i).collect();
    h.send(CanFrame::fd(REPLY_ID, false, true, &out).unwrap()).await.expect("send FD");
    let (k, n) = raw_read_id(&peer, REPLY_ID);
    assert_eq!(n, 72, "an FD frame comes as a 72-byte canfd_frame");
    assert_eq!(k.len, 48);
    assert_ne!(k.flags & CANFD_BRS, 0);
    assert_eq!(&k.data[..48], &out[..]);
    h.send(CanFrame::new(REPLY_ID, &[9, 8, 7]).unwrap()).await.expect("send classic");
    let (k, n) = raw_read_id(&peer, REPLY_ID);
    assert_eq!(n, 16, "a classic frame stays a 16-byte can_frame");
    assert_eq!((k.len, &k.data[..3]), (3, &[9u8, 8, 7][..]));

    h.close().await;
}

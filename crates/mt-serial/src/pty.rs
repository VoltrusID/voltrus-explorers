//! Pseudo-terminal pairs for virtual serial ports.

use std::ffi::CStr;
use std::io;
use std::os::raw::{c_char, c_int};

/// An open master/slave pair. The caller owns both descriptors.
///
/// Keep `slave_fd` open for as long as the port should exist: once the last slave is closed the
/// master reads end-of-file and the port is dead before another program has opened `slave_path`.
#[derive(Debug)]
pub struct PtyPair {
    pub master_fd: c_int,
    pub slave_fd: c_int,
    pub slave_path: String,
}

pub fn open_pair() -> io::Result<PtyPair> {
    let mut master_fd: c_int = -1;
    let mut slave_fd: c_int = -1;
    let mut name_buf = [0u8; 256];
    let rc = unsafe {
        libc::openpty(
            &mut master_fd,
            &mut slave_fd,
            name_buf.as_mut_ptr() as *mut c_char,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        )
    };
    if rc == -1 {
        return Err(io::Error::last_os_error());
    }
    let slave_path = unsafe { CStr::from_ptr(name_buf.as_ptr() as *const c_char) }
        .to_string_lossy()
        .into_owned();
    Ok(PtyPair { master_fd, slave_fd, slave_path })
}

/// Put a terminal fd into raw mode (8 bits, no translation, no echo, no line editing).
pub fn set_raw(fd: c_int) -> io::Result<()> {
    let mut termios = std::mem::MaybeUninit::<libc::termios>::uninit();
    if unsafe { libc::tcgetattr(fd, termios.as_mut_ptr()) } == -1 {
        return Err(io::Error::last_os_error());
    }
    let mut termios = unsafe { termios.assume_init() };
    unsafe { libc::cfmakeraw(&mut termios) };
    if unsafe { libc::tcsetattr(fd, libc::TCSANOW, &termios) } == -1 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

impl PtyPair {
    /// Make the pair a binary serial line.
    ///
    /// `openpty` leaves the cooked line discipline: ICRNL and ONLCR rewrite 0x0D and 0x0A, ICANON
    /// holds input back until a newline, and `tcdrain` on the master (what `flush()` calls) never
    /// returns until a program reads the slave. The two ends share one termios, so one successful
    /// `tcsetattr` is enough; the App Sandbox refuses it with EPERM on some side, so both are tried.
    pub fn make_raw(&self) -> io::Result<()> {
        let slave = set_raw(self.slave_fd);
        if slave.is_ok() {
            return Ok(());
        }
        let master = set_raw(self.master_fd);
        if master.is_ok() {
            return Ok(());
        }
        Err(io::Error::other(format!(
            "setting the pty to raw mode failed (slave: {}, master: {})",
            slave.unwrap_err(),
            master.unwrap_err()
        )))
    }
}

/// Close a descriptor returned by [`open_pair`].
pub fn close_fd(fd: c_int) {
    unsafe { libc::close(fd) };
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set_nonblocking(fd: c_int) {
        unsafe {
            let flags = libc::fcntl(fd, libc::F_GETFL);
            assert!(libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) != -1);
        }
    }

    fn read_once(fd: c_int) -> io::Result<Vec<u8>> {
        let mut buf = [0u8; 64];
        let n = unsafe { libc::read(fd, buf.as_mut_ptr() as *mut libc::c_void, buf.len()) };
        if n < 0 {
            Err(io::Error::last_os_error())
        } else {
            Ok(buf[..n as usize].to_vec())
        }
    }

    // Why `open_pair` callers keep the slave open: with it closed the master reports a hang-up.
    #[test]
    fn closing_the_last_slave_hangs_up_the_master() {
        let pair = open_pair().unwrap();
        set_nonblocking(pair.master_fd);
        close_fd(pair.slave_fd);
        let result = read_once(pair.master_fd);
        let hung_up = match &result {
            Ok(bytes) => bytes.is_empty(),
            Err(e) => e.kind() != io::ErrorKind::WouldBlock,
        };
        assert!(hung_up, "expected end-of-file or EIO, got {result:?}");
        close_fd(pair.master_fd);
    }

    #[test]
    fn idle_pair_with_the_slave_held_open_does_not_hang_up() {
        let pair = open_pair().unwrap();
        set_nonblocking(pair.master_fd);
        let err = read_once(pair.master_fd).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::WouldBlock, "master saw a hang-up: {err}");
        close_fd(pair.slave_fd);
        close_fd(pair.master_fd);
    }

    fn open_slave_path(pair: &PtyPair) -> c_int {
        let path = std::ffi::CString::new(pair.slave_path.clone()).unwrap();
        let fd = unsafe { libc::open(path.as_ptr(), libc::O_RDWR | libc::O_NOCTTY) };
        assert!(fd >= 0, "could not open {}: {}", pair.slave_path, io::Error::last_os_error());
        fd
    }

    fn write_all(fd: c_int, bytes: &[u8]) {
        let n = unsafe { libc::write(fd, bytes.as_ptr() as *const libc::c_void, bytes.len()) };
        assert_eq!(n, bytes.len() as isize);
    }

    // The line discipline hands bytes over asynchronously.
    fn read_exactly(fd: c_int, len: usize) -> Vec<u8> {
        set_nonblocking(fd);
        let mut got = Vec::new();
        for _ in 0..100 {
            if let Ok(chunk) = read_once(fd) {
                got.extend(chunk);
            }
            if got.len() >= len {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        got
    }

    // 0x0D, 0x0A and 0x03 are rewritten or acted on by the cooked line discipline.
    const BINARY: [u8; 5] = [0xAA, 0x0D, 0x0A, 0x03, 0x11];

    #[test]
    fn raw_pair_carries_binary_from_another_program_to_the_master() {
        let pair = open_pair().unwrap();
        pair.make_raw().unwrap();
        let other = open_slave_path(&pair);
        write_all(other, &BINARY);
        assert_eq!(read_exactly(pair.master_fd, BINARY.len()), BINARY);
        close_fd(other);
        close_fd(pair.slave_fd);
        close_fd(pair.master_fd);
    }

    #[test]
    fn raw_pair_carries_binary_from_the_master_to_another_program() {
        let pair = open_pair().unwrap();
        pair.make_raw().unwrap();
        let other = open_slave_path(&pair);
        write_all(pair.master_fd, &BINARY);
        assert_eq!(read_exactly(other, BINARY.len()), BINARY);
        close_fd(other);
        close_fd(pair.slave_fd);
        close_fd(pair.master_fd);
    }

    // `flush()` on the master stream is tcdrain; on a cooked pty it never returns while nobody reads the slave.
    #[test]
    fn draining_the_master_returns_even_when_nobody_reads_the_slave() {
        let pair = open_pair().unwrap();
        pair.make_raw().unwrap();
        write_all(pair.master_fd, &[1, 2, 3]);
        let master = pair.master_fd;
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let rc = unsafe { libc::tcdrain(master) };
            let _ = tx.send(rc);
        });
        let rc = rx.recv_timeout(std::time::Duration::from_secs(5)).expect("tcdrain blocked");
        assert_eq!(rc, 0);
        close_fd(pair.slave_fd);
        close_fd(pair.master_fd);
    }
}

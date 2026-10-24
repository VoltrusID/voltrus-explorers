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

    #[test]
    fn another_program_can_open_the_slave_path_and_reach_the_master() {
        let pair = open_pair().unwrap();
        // Raw mode so the bytes arrive unchanged on either side of the pair.
        let mut termios = std::mem::MaybeUninit::<libc::termios>::uninit();
        unsafe {
            assert_eq!(libc::tcgetattr(pair.slave_fd, termios.as_mut_ptr()), 0);
            let mut t = termios.assume_init();
            libc::cfmakeraw(&mut t);
            assert_eq!(libc::tcsetattr(pair.slave_fd, libc::TCSANOW, &t), 0);
        }
        let path = std::ffi::CString::new(pair.slave_path.clone()).unwrap();
        let other = unsafe { libc::open(path.as_ptr(), libc::O_RDWR | libc::O_NOCTTY) };
        assert!(other >= 0, "could not open {}: {}", pair.slave_path, io::Error::last_os_error());
        let payload = [0xAAu8, 0x0D, 0x0A, 0x03];
        let written = unsafe { libc::write(other, payload.as_ptr() as *const libc::c_void, payload.len()) };
        assert_eq!(written, payload.len() as isize);
        set_nonblocking(pair.master_fd);
        // The line discipline hands the bytes over asynchronously.
        let mut got = Vec::new();
        for _ in 0..50 {
            if let Ok(chunk) = read_once(pair.master_fd) {
                got.extend(chunk);
            }
            if got.len() >= payload.len() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        assert_eq!(got, payload);
        close_fd(other);
        close_fd(pair.slave_fd);
        close_fd(pair.master_fd);
    }
}

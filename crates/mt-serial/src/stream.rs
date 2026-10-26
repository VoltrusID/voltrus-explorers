//! Opening a port as an async byte stream, for monitors that read and write
//! raw bytes (no request/response framing).

use std::io;

use tokio_serial::SerialStream;

use crate::types::SerialConfig;

/// Whether `path`, after following links, is a pseudo-terminal: a virtual
/// port, where line settings such as the baud rate mean nothing.
pub fn is_pseudo_terminal(path: &str) -> bool {
    let resolved = std::fs::canonicalize(path)
        .map(|resolved| resolved.to_string_lossy().into_owned())
        .unwrap_or_else(|_| path.to_owned());
    resolved.starts_with("/dev/ttys") || resolved.starts_with("/dev/pts/")
}

/// Open `config`'s port for async reads and writes.
///
/// macOS refuses a baud rate on a pseudo-terminal (the speed ioctl answers
/// ENOTTY), so a virtual port opens without one there. Input that queued up
/// before the port was opened (a device talking to nobody) is discarded.
/// Errors keep their kind: `NotFound` for a missing device, `InvalidInput`
/// for bad settings.
pub fn open_stream(config: &SerialConfig) -> io::Result<SerialStream> {
    use tokio_serial::SerialPort as _;

    let mut builder = config
        .to_serial_port_builder()
        .map_err(|message| io::Error::new(io::ErrorKind::InvalidInput, message))?;
    if cfg!(any(target_os = "macos", target_os = "ios")) && is_pseudo_terminal(&config.port_name) {
        builder = builder.baud_rate(0);
    }
    let stream = SerialStream::open(&builder).map_err(io::Error::from)?;
    let _ = stream.clear(tokio_serial::ClearBuffer::Input);
    Ok(stream)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn device_paths_are_not_pseudo_terminals() {
        assert!(!is_pseudo_terminal("/dev/cu.usbserial-0001"));
        assert!(!is_pseudo_terminal("/dev/tty.usbmodem14101"));
        assert!(!is_pseudo_terminal("/dev/ttyS0"));
        assert!(is_pseudo_terminal("/dev/pts/3"));
    }

    #[test]
    fn a_missing_port_is_not_found() {
        let config = SerialConfig {
            port_name: "/dev/cu.voltrus-does-not-exist".into(),
            baud_rate: 9600,
            data_bits: 8,
            stop_bits: 1,
            parity: crate::Parity::None,
            flow_control: crate::FlowControl::None,
            timeout_ms: 100,
        };
        let error = open_stream(&config).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::NotFound, "{error}");
    }

    #[test]
    fn bad_settings_are_invalid_input() {
        let config = SerialConfig {
            port_name: "/dev/null".into(),
            baud_rate: 9600,
            data_bits: 9,
            stop_bits: 1,
            parity: crate::Parity::None,
            flow_control: crate::FlowControl::None,
            timeout_ms: 100,
        };
        let error = open_stream(&config).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidInput, "{error}");
    }
}

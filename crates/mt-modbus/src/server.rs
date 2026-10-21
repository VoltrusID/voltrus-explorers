use std::io;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use tokio::net::{TcpListener, TcpStream};
use tokio_modbus::prelude::*;
use tokio_modbus::server::Service;
use tokio_modbus::server::tcp::{Server, accept_tcp_connection};
#[cfg(feature = "serial")]
use tokio_serial::SerialStream;

#[cfg(feature = "serial")]
use crate::types::RtuParity;
use crate::types::SlaveLogEntry;

/// Shared register image backing the slave server.
pub struct SlaveState {
    pub registers: Vec<u16>,
}

struct SlaveService {
    data: Arc<Mutex<SlaveState>>,
    log_tx: tokio::sync::mpsc::UnboundedSender<SlaveLogEntry>,
}

impl Service for SlaveService {
    type Request = Request<'static>;
    type Response = Response;
    type Exception = ExceptionCode;
    type Future = std::future::Ready<Result<Self::Response, Self::Exception>>;

    fn call(&self, req: Self::Request) -> Self::Future {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);

        let response = match &req {
            Request::ReadHoldingRegisters(req_addr, qty) => {
                let data = self.data.lock().unwrap();
                let addr = *req_addr as usize;
                let qty = *qty as usize;
                if addr.saturating_add(qty) > data.registers.len() {
                    let _ = self.log_tx.send(SlaveLogEntry {
                        timestamp: now,
                        function_code: 3,
                        address: addr as u16,
                        quantity: qty as u16,
                        message: format!(
                            "Read rejected: {}..{} out of range (max {})",
                            addr,
                            addr + qty,
                            data.registers.len()
                        ),
                    });
                    return std::future::ready(Err(ExceptionCode::IllegalDataAddress));
                }
                let regs = data.registers[addr..addr + qty].to_vec();
                let _ = self.log_tx.send(SlaveLogEntry {
                    timestamp: now,
                    function_code: 3,
                    address: addr as u16,
                    quantity: qty as u16,
                    message: format!("Read holding regs {}..{} ({} words)", addr, addr + regs.len(), regs.len()),
                });
                Response::ReadHoldingRegisters(regs)
            }
            Request::WriteSingleRegister(req_addr, word) => {
                let mut data = self.data.lock().unwrap();
                let addr = *req_addr as usize;
                if addr < data.registers.len() {
                    data.registers[addr] = *word;
                    let _ = self.log_tx.send(SlaveLogEntry {
                        timestamp: now,
                        function_code: 6,
                        address: *req_addr,
                        quantity: 1,
                        message: format!("Write single reg {} = {}", addr, word),
                    });
                    Response::WriteSingleRegister(*req_addr, *word)
                } else {
                    let _ = self.log_tx.send(SlaveLogEntry {
                        timestamp: now,
                        function_code: 6,
                        address: *req_addr,
                        quantity: 1,
                        message: format!("Write rejected: addr {} out of range", addr),
                    });
                    return std::future::ready(Err(ExceptionCode::IllegalDataAddress));
                }
            }
            Request::WriteMultipleRegisters(req_addr, words) => {
                let mut data = self.data.lock().unwrap();
                let addr = *req_addr as usize;
                let end = addr + words.len();
                if end <= data.registers.len() {
                    data.registers[addr..end].copy_from_slice(words);
                    let _ = self.log_tx.send(SlaveLogEntry {
                        timestamp: now,
                        function_code: 16,
                        address: *req_addr,
                        quantity: words.len() as u16,
                        message: format!("Write {} regs at {}..{}", words.len(), addr, end),
                    });
                    Response::WriteMultipleRegisters(*req_addr, words.len() as u16)
                } else {
                    let _ = self.log_tx.send(SlaveLogEntry {
                        timestamp: now,
                        function_code: 16,
                        address: *req_addr,
                        quantity: words.len() as u16,
                        message: format!("Write rejected: {}..{} out of range (max {})", addr, end, data.registers.len()),
                    });
                    return std::future::ready(Err(ExceptionCode::IllegalDataAddress));
                }
            }
            _ => {
                return std::future::ready(Err(ExceptionCode::IllegalFunction));
            }
        };

        std::future::ready(Ok(response))
    }
}

/// Start the slave TCP server. Returns `(SlaveState handle, log receiver)`.
pub async fn start_slave_server(
    port: u16,
    bind_addr: &str,
    register_count: u16,
) -> anyhow::Result<(
    Arc<Mutex<SlaveState>>,
    tokio::sync::mpsc::UnboundedReceiver<SlaveLogEntry>,
    tokio::sync::watch::Sender<bool>,
)> {
    let addr: SocketAddr = format!("{bind_addr}:{port}").parse()?;
    let listener = TcpListener::bind(addr).await?;

    let state = Arc::new(Mutex::new(SlaveState {
        registers: vec![0u16; register_count as usize],
    }));

    let (log_tx, log_rx) = tokio::sync::mpsc::unbounded_channel();
    let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);

    let data = state.clone();
    let log_tx_clone = log_tx.clone();

    let new_service = move |_socket_addr: SocketAddr| -> io::Result<Option<SlaveService>> {
        Ok(Some(SlaveService {
            data: data.clone(),
            log_tx: log_tx_clone.clone(),
        }))
    };

    tokio::spawn(async move {
        let server = Server::new(listener);

        let on_connected = |stream: TcpStream, socket_addr: SocketAddr| {
            let new_service = &new_service;
            async move { accept_tcp_connection(stream, socket_addr, new_service) }
        };

        let on_process_error = |err: io::Error| {
            tracing::error!("Slave connection error: {err}");
        };

        let shutdown_future = Box::pin(async move {
            let mut rx = shutdown_rx;
            loop {
                if *rx.borrow_and_update() {
                    break;
                }
                if rx.changed().await.is_err() {
                    break;
                }
            }
        });

        tracing::info!("Modbus slave server listening on {addr}");
        if let Err(e) = server
            .serve_until(&on_connected, on_process_error, shutdown_future)
            .await
        {
            tracing::error!("Slave server error: {e}");
        }
        tracing::info!("Slave server stopped");
        let _ = log_tx.send(SlaveLogEntry {
            timestamp: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_millis() as u64)
                .unwrap_or(0),
            function_code: 0,
            address: 0,
            quantity: 0,
            message: "Server stopped".into(),
        });
    });

    Ok((state, log_rx, shutdown_tx))
}

/// Start the slave RTU server on a serial port. Returns `(SlaveState handle, log receiver)`.
#[cfg(feature = "serial")]
pub async fn start_slave_server_rtu(
    port_name: &str,
    baud_rate: u32,
    data_bits: u8,
    stop_bits: u8,
    parity: RtuParity,
    register_count: u16,
) -> anyhow::Result<(
    Arc<Mutex<SlaveState>>,
    tokio::sync::mpsc::UnboundedReceiver<SlaveLogEntry>,
    tokio::sync::watch::Sender<bool>,
)> {
    let port_name = port_name.to_owned();
    let builder = tokio_serial::new(&port_name, baud_rate)
        .data_bits(match data_bits {
            5 => serialport::DataBits::Five,
            6 => serialport::DataBits::Six,
            7 => serialport::DataBits::Seven,
            _ => serialport::DataBits::Eight,
        })
        .stop_bits(match stop_bits {
            2 => serialport::StopBits::Two,
            _ => serialport::StopBits::One,
        })
        .parity(match parity {
            RtuParity::None => serialport::Parity::None,
            RtuParity::Odd => serialport::Parity::Odd,
            RtuParity::Even => serialport::Parity::Even,
        });

    let stream = SerialStream::open(&builder)?;

    let state = Arc::new(Mutex::new(SlaveState {
        registers: vec![0u16; register_count as usize],
    }));

    let (log_tx, log_rx) = tokio::sync::mpsc::unbounded_channel();
    let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);

    let data = state.clone();
    let log_tx_clone = log_tx.clone();

    let service = SlaveService {
        data,
        log_tx: log_tx_clone.clone(),
    };

    tokio::spawn(async move {
        let _ = log_tx_clone.send(SlaveLogEntry {
            timestamp: SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0),
            function_code: 0,
            address: 0,
            quantity: 0,
            message: format!("RTU slave started on {} @ {} baud", port_name, baud_rate),
        });

        let server = tokio_modbus::server::rtu::Server::new(stream);

        let shutdown_future = Box::pin(async move {
            let mut rx = shutdown_rx;
            loop {
                if *rx.borrow_and_update() {
                    break;
                }
                if rx.changed().await.is_err() {
                    break;
                }
            }
        });

        tracing::info!("Modbus RTU slave server started on {}", port_name);
        let _ = server.serve_until(service, shutdown_future).await;
        tracing::info!("RTU slave server stopped");
        let _ = log_tx_clone.send(SlaveLogEntry {
            timestamp: SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0),
            function_code: 0,
            address: 0,
            quantity: 0,
            message: "RTU server stopped".into(),
        });
    });

    Ok((state, log_rx, shutdown_tx))
}

/// Start the slave RTU server on a virtual serial port (PTY pair).
/// Returns `(SlaveState handle, log receiver, shutdown sender, slave PTY path, slave fd)`.
/// The slave fd must be kept open and used by the in-app master via `dup()`.
#[cfg(all(feature = "serial", unix))]
pub async fn start_slave_server_rtu_virtual(
    register_count: u16,
) -> anyhow::Result<(
    Arc<Mutex<SlaveState>>,
    tokio::sync::mpsc::UnboundedReceiver<SlaveLogEntry>,
    tokio::sync::watch::Sender<bool>,
    String,
    std::os::fd::RawFd,
)> {
    use std::ffi::CStr;
    use std::os::unix::io::FromRawFd;

    let mut master_fd: libc::c_int = -1;
    let mut slave_fd: libc::c_int = -1;
    let mut name_buf = [0u8; 256];

    let result = unsafe {
        libc::openpty(
            &mut master_fd,
            &mut slave_fd,
            name_buf.as_mut_ptr() as *mut libc::c_char,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        )
    };

    if result == -1 {
        return Err(std::io::Error::last_os_error().into());
    }

    let slave_path = unsafe { CStr::from_ptr(name_buf.as_ptr() as *const libc::c_char) }
        .to_string_lossy()
        .into_owned();

    // Keep slave_fd open — in-app master uses dup() to connect

    // master fd → TTYPort → SerialStream
    let tty_port = unsafe { serialport::TTYPort::from_raw_fd(master_fd) };
    let stream = tokio_serial::SerialStream::try_from(tty_port)?;

    let state = Arc::new(Mutex::new(SlaveState {
        registers: vec![0u16; register_count as usize],
    }));

    let (log_tx, log_rx) = tokio::sync::mpsc::unbounded_channel();
    let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);

    let data = state.clone();
    let log_tx_clone = log_tx.clone();

    let service = SlaveService {
        data,
        log_tx: log_tx_clone.clone(),
    };

    let slave_path_display = slave_path.clone();
    tokio::spawn(async move {
        let _ = log_tx_clone.send(SlaveLogEntry {
            timestamp: SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0),
            function_code: 0,
            address: 0,
            quantity: 0,
            message: format!("Virtual RTU slave started at {}", slave_path_display),
        });

        let server = tokio_modbus::server::rtu::Server::new(stream);

        let shutdown_future = Box::pin(async move {
            let mut rx = shutdown_rx;
            loop {
                if *rx.borrow_and_update() {
                    break;
                }
                if rx.changed().await.is_err() {
                    break;
                }
            }
        });

        tracing::info!("Virtual RTU slave started at {}", slave_path_display);
        let _ = server.serve_until(service, shutdown_future).await;
        tracing::info!("Virtual RTU slave stopped");
        let _ = log_tx_clone.send(SlaveLogEntry {
            timestamp: SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0),
            function_code: 0,
            address: 0,
            quantity: 0,
            message: "Virtual RTU server stopped".into(),
        });
    });

    Ok((state, log_rx, shutdown_tx, slave_path, slave_fd))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio_modbus::server::Service;
    use std::borrow::Cow;

    fn make_service(register_count: usize) -> (Arc<Mutex<SlaveState>>, SlaveService) {
        let state = Arc::new(Mutex::new(SlaveState {
            registers: vec![0u16; register_count],
        }));
        let (log_tx, _log_rx) = tokio::sync::mpsc::unbounded_channel();
        let service = SlaveService {
            data: state.clone(),
            log_tx,
        };
        (state, service)
    }

    fn run<F: std::future::Future>(fut: F) -> F::Output {
        // std::future::Ready completes immediately, so a minimal waker suffices
        use std::task::{Context, Poll, RawWaker, RawWakerVTable, Waker};
        fn dummy_raw() -> RawWaker {
            RawWaker::new(std::ptr::null(), &VTABLE)
        }
        static VTABLE: RawWakerVTable = RawWakerVTable::new(
            |_| dummy_raw(),
            |_| {},
            |_| {},
            |_| {},
        );
        let waker = unsafe { Waker::from_raw(dummy_raw()) };
        let mut cx = Context::from_waker(&waker);
        let mut pinned = std::pin::pin!(fut);
        match pinned.as_mut().poll(&mut cx) {
            Poll::Ready(val) => val,
            Poll::Pending => panic!("Future did not complete immediately"),
        }
    }

    #[test]
    fn test_read_holding_registers_basic() {
        let (state, service) = make_service(10);
        {
            let mut data = state.lock().unwrap();
            data.registers[0] = 100;
            data.registers[1] = 200;
            data.registers[2] = 300;
        }
        let result = run(service.call(Request::ReadHoldingRegisters(0, 3))).unwrap();
        match result {
            Response::ReadHoldingRegisters(regs) => assert_eq!(regs, vec![100, 200, 300]),
            _ => panic!("Expected ReadHoldingRegisters response"),
        }
    }

    #[test]
    fn test_write_single_register() {
        let (state, service) = make_service(10);
        let result = run(service.call(Request::WriteSingleRegister(5, 42))).unwrap();
        match result {
            Response::WriteSingleRegister(addr, word) => {
                assert_eq!(addr, 5);
                assert_eq!(word, 42);
            }
            _ => panic!("Expected WriteSingleRegister response"),
        }
        let data = state.lock().unwrap();
        assert_eq!(data.registers[5], 42);
    }

    #[test]
    fn test_write_multiple_registers() {
        let (state, service) = make_service(10);
        let result = run(service.call(Request::WriteMultipleRegisters(0, Cow::Borrowed(&[10, 20, 30])))).unwrap();
        match result {
            Response::WriteMultipleRegisters(addr, count) => {
                assert_eq!(addr, 0);
                assert_eq!(count, 3);
            }
            _ => panic!("Expected WriteMultipleRegisters response"),
        }
        let data = state.lock().unwrap();
        assert_eq!(data.registers[0], 10);
        assert_eq!(data.registers[1], 20);
        assert_eq!(data.registers[2], 30);
    }

    #[test]
    fn test_write_out_of_range_returns_illegal_address() {
        let (_state, service) = make_service(100);
        let result = run(service.call(Request::WriteSingleRegister(999, 42)));
        assert_eq!(result, Err(ExceptionCode::IllegalDataAddress));
    }

    #[test]
    fn test_unsupported_function_returns_illegal_function() {
        let (_state, service) = make_service(10);
        let result = run(service.call(Request::ReadCoils(0, 1)));
        assert_eq!(result, Err(ExceptionCode::IllegalFunction));
    }

    // ── Expanded coverage ──

    #[test]
    fn test_read_holding_registers_out_of_range_returns_illegal_address() {
        let (_state, service) = make_service(5);
        // Request registers starting at address 10, only 5 exist:
        // Modbus requires an address exception, never an empty response.
        let result = run(service.call(Request::ReadHoldingRegisters(10, 3)));
        assert_eq!(result, Err(ExceptionCode::IllegalDataAddress));
    }

    #[test]
    fn test_read_holding_registers_partial_range() {
        let (_state, service) = make_service(10);
        // Request 5 registers starting at 7, but only 3 exist (7, 8, 9):
        // a partial read must also be rejected as an address exception.
        let result = run(service.call(Request::ReadHoldingRegisters(7, 5)));
        assert_eq!(result, Err(ExceptionCode::IllegalDataAddress));
    }

    #[test]
    fn test_write_multiple_registers_out_of_range() {
        let (_state, service) = make_service(5);
        let result = run(service.call(Request::WriteMultipleRegisters(
            3, Cow::Borrowed(&[10, 20, 30]), // 3-6 exceeds 5 registers
        )));
        assert_eq!(result, Err(ExceptionCode::IllegalDataAddress));
    }

    #[test]
    fn test_write_single_at_last_register() {
        let (state, service) = make_service(10);
        let result = run(service.call(Request::WriteSingleRegister(9, 9999))).unwrap();
        match result {
            Response::WriteSingleRegister(addr, word) => {
                assert_eq!(addr, 9);
                assert_eq!(word, 9999);
            }
            _ => panic!("Expected WriteSingleRegister response"),
        }
        let data = state.lock().unwrap();
        assert_eq!(data.registers[9], 9999);
    }

    #[test]
    fn test_read_holding_zero_quantity() {
        let (_state, service) = make_service(10);
        let result = run(service.call(Request::ReadHoldingRegisters(0, 0))).unwrap();
        match result {
            Response::ReadHoldingRegisters(regs) => assert!(regs.is_empty()),
            _ => panic!("Expected ReadHoldingRegisters response"),
        }
    }

    #[test]
    fn test_read_input_registers_returns_illegal_function() {
        let (_state, service) = make_service(10);
        let result = run(service.call(Request::ReadInputRegisters(0, 1)));
        assert_eq!(result, Err(ExceptionCode::IllegalFunction));
    }
}

use std::io;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};
use std::time::{SystemTime, UNIX_EPOCH};

use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{mpsc, watch};
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

/// Most registers one Read Holding Registers request may ask for (§6.3).
const MAX_READ_REGISTERS: u16 = 125;
/// Most registers one Write Multiple Registers request may carry (§6.12).
const MAX_WRITE_REGISTERS: u16 = 123;

type LogSender = mpsc::UnboundedSender<SlaveLogEntry>;

/// What [`start_slave_server`] and friends hand back to the caller.
pub type SlaveHandles = (
    Arc<Mutex<SlaveState>>,
    mpsc::UnboundedReceiver<SlaveLogEntry>,
    watch::Sender<bool>,
);

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn status_entry(message: String) -> SlaveLogEntry {
    SlaveLogEntry { timestamp: now_ms(), function_code: 0, address: 0, quantity: 0, message }
}

/// How requests are addressed on the transport the service runs on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
// `Rtu` is only constructed by the serial-feature start functions.
#[cfg_attr(not(feature = "serial"), allow(dead_code))]
enum Transport {
    /// Multi-drop serial bus: only the configured unit answers, unit 0 is
    /// broadcast (writes are applied, nothing is ever sent back).
    Rtu,
    /// TCP: the device is addressed by IP, so besides the configured unit the
    /// "direct addressing" IDs 0xFF and 0x00 are accepted; any other unit ID
    /// is answered with exception 0x0B (Gateway Target Device Failed to Respond).
    Tcp,
}

enum Addressed {
    Me,
    Broadcast,
    Other,
}

struct SlaveService {
    data: Arc<Mutex<SlaveState>>,
    log_tx: LogSender,
    unit_id: u8,
    transport: Transport,
}

impl SlaveService {
    fn new(data: Arc<Mutex<SlaveState>>, log_tx: LogSender, unit_id: u8, transport: Transport) -> Self {
        Self { data, log_tx, unit_id, transport }
    }

    fn addressed(&self, slave: SlaveId) -> Addressed {
        if slave == self.unit_id {
            return Addressed::Me;
        }
        match self.transport {
            Transport::Rtu if slave == 0 => Addressed::Broadcast,
            Transport::Tcp if slave == 0 || slave == 0xFF => Addressed::Me,
            _ => Addressed::Other,
        }
    }

    fn log(&self, function_code: u8, address: u16, quantity: u16, message: String) {
        let _ = self.log_tx.send(SlaveLogEntry { timestamp: now_ms(), function_code, address, quantity, message });
    }

    fn dispatch(&self, slave: SlaveId, request: &Request<'_>) -> Result<Option<Response>, ExceptionCode> {
        match self.addressed(slave) {
            Addressed::Me => self.handle(request).map(Some),
            Addressed::Broadcast => {
                // Broadcast writes are executed by every slave; nobody answers a broadcast.
                if matches!(request, Request::WriteSingleRegister(..) | Request::WriteMultipleRegisters(..)) {
                    let _ = self.handle(request);
                }
                Ok(None)
            }
            Addressed::Other => match self.transport {
                // Another device's traffic on a shared bus: stay silent.
                Transport::Rtu => Ok(None),
                Transport::Tcp => {
                    self.log(0, 0, 0, format!("Request for unit {slave} rejected (this slave is unit {})", self.unit_id));
                    Err(ExceptionCode::GatewayTargetDevice)
                }
            },
        }
    }

    fn handle(&self, req: &Request<'_>) -> Result<Response, ExceptionCode> {
        match req {
            Request::ReadHoldingRegisters(req_addr, qty) => {
                // Quantity is checked before the address (Modbus Application Protocol v1.1b3, §6.3):
                // a count outside 1..=125 is Illegal Data Value (0x03), whatever the address.
                if !(1..=MAX_READ_REGISTERS).contains(qty) {
                    self.log(3, *req_addr, *qty, format!("Read rejected: quantity {qty} outside 1..={MAX_READ_REGISTERS}"));
                    return Err(ExceptionCode::IllegalDataValue);
                }
                let data = self.data.lock().unwrap();
                let addr = *req_addr as usize;
                let qty = *qty as usize;
                if addr.saturating_add(qty) > data.registers.len() {
                    self.log(
                        3,
                        *req_addr,
                        qty as u16,
                        format!("Read rejected: {}..{} out of range (max {})", addr, addr + qty, data.registers.len()),
                    );
                    return Err(ExceptionCode::IllegalDataAddress);
                }
                let regs = data.registers[addr..addr + qty].to_vec();
                self.log(
                    3,
                    *req_addr,
                    qty as u16,
                    format!("Read holding regs {}..{} ({} words)", addr, addr + regs.len(), regs.len()),
                );
                Ok(Response::ReadHoldingRegisters(regs))
            }
            Request::WriteSingleRegister(req_addr, word) => {
                let mut data = self.data.lock().unwrap();
                let addr = *req_addr as usize;
                if addr < data.registers.len() {
                    data.registers[addr] = *word;
                    self.log(6, *req_addr, 1, format!("Write single reg {} = {}", addr, word));
                    Ok(Response::WriteSingleRegister(*req_addr, *word))
                } else {
                    self.log(6, *req_addr, 1, format!("Write rejected: addr {} out of range", addr));
                    Err(ExceptionCode::IllegalDataAddress)
                }
            }
            Request::WriteMultipleRegisters(req_addr, words) => {
                // §6.12: 1..=123 registers; checked before the address.
                if !(1..=MAX_WRITE_REGISTERS as usize).contains(&words.len()) {
                    self.log(
                        16,
                        *req_addr,
                        words.len().min(u16::MAX as usize) as u16,
                        format!("Write rejected: quantity {} outside 1..={MAX_WRITE_REGISTERS}", words.len()),
                    );
                    return Err(ExceptionCode::IllegalDataValue);
                }
                let mut data = self.data.lock().unwrap();
                let addr = *req_addr as usize;
                let end = addr + words.len();
                if end <= data.registers.len() {
                    data.registers[addr..end].copy_from_slice(words);
                    self.log(16, *req_addr, words.len() as u16, format!("Write {} regs at {}..{}", words.len(), addr, end));
                    Ok(Response::WriteMultipleRegisters(*req_addr, words.len() as u16))
                } else {
                    self.log(
                        16,
                        *req_addr,
                        words.len() as u16,
                        format!("Write rejected: {}..{} out of range (max {})", addr, end, data.registers.len()),
                    );
                    Err(ExceptionCode::IllegalDataAddress)
                }
            }
            _ => Err(ExceptionCode::IllegalFunction),
        }
    }
}

impl Service for SlaveService {
    type Request = SlaveRequest<'static>;
    /// `None` = send no response (broadcast, or another unit's request on a serial bus).
    type Response = Option<Response>;
    type Exception = ExceptionCode;
    type Future = std::future::Ready<Result<Self::Response, Self::Exception>>;

    fn call(&self, req: Self::Request) -> Self::Future {
        std::future::ready(self.dispatch(req.slave, &req.request))
    }
}

/// Resolve once the shutdown flag is set (or its sender is gone).
async fn wait_shutdown(mut rx: watch::Receiver<bool>) {
    let _ = rx.wait_for(|stop| *stop).await;
}

/// A TCP connection that reads EOF and refuses writes once shutdown is signalled.
///
/// tokio-modbus spawns one task per accepted connection and only cancels the
/// accept loop on shutdown, so without this, already connected masters would
/// keep talking to the register image of a server the user stopped.
struct StopOnShutdown<T> {
    inner: T,
    stop: Pin<Box<dyn Future<Output = ()> + Send>>,
    stopped: bool,
}

impl<T> StopOnShutdown<T> {
    fn new(inner: T, shutdown: watch::Receiver<bool>) -> Self {
        Self { inner, stop: Box::pin(wait_shutdown(shutdown)), stopped: false }
    }

    fn poll_stopped(&mut self, cx: &mut Context<'_>) -> bool {
        if !self.stopped {
            self.stopped = self.stop.as_mut().poll(cx).is_ready();
        }
        self.stopped
    }
}

impl<T: AsyncRead + Unpin> AsyncRead for StopOnShutdown<T> {
    fn poll_read(self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &mut ReadBuf<'_>) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        if this.poll_stopped(cx) {
            return Poll::Ready(Ok(())); // EOF ends the connection task
        }
        Pin::new(&mut this.inner).poll_read(cx, buf)
    }
}

impl<T: AsyncWrite + Unpin> AsyncWrite for StopOnShutdown<T> {
    fn poll_write(self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &[u8]) -> Poll<io::Result<usize>> {
        let this = self.get_mut();
        if this.poll_stopped(cx) {
            return Poll::Ready(Err(io::ErrorKind::BrokenPipe.into()));
        }
        Pin::new(&mut this.inner).poll_write(cx, buf)
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.get_mut().inner).poll_flush(cx)
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.get_mut().inner).poll_shutdown(cx)
    }
}

fn new_state(register_count: u16) -> Arc<Mutex<SlaveState>> {
    Arc::new(Mutex::new(SlaveState { registers: vec![0u16; register_count as usize] }))
}

/// Start the slave TCP server. Returns `(SlaveState handle, log receiver, shutdown sender)`.
///
/// The slave answers `unit_id` (and the direct-addressing IDs 0xFF/0x00); other
/// unit IDs get exception 0x0B. Sending `true` on the shutdown sender stops the
/// listener and closes every connected master.
pub async fn start_slave_server(
    port: u16,
    bind_addr: &str,
    register_count: u16,
    unit_id: u8,
) -> anyhow::Result<SlaveHandles> {
    let addr: SocketAddr = format!("{bind_addr}:{port}").parse()?;
    let listener = TcpListener::bind(addr).await?;

    let state = new_state(register_count);
    let (log_tx, log_rx) = mpsc::unbounded_channel();
    let (shutdown_tx, shutdown_rx) = watch::channel(false);

    let data = state.clone();
    let service_log_tx = log_tx.clone();
    let connection_shutdown = shutdown_rx.clone();

    let new_service = move |_socket_addr: SocketAddr| -> io::Result<Option<SlaveService>> {
        Ok(Some(SlaveService::new(data.clone(), service_log_tx.clone(), unit_id, Transport::Tcp)))
    };

    tokio::spawn(async move {
        let server = Server::new(listener);

        let on_connected = |stream: TcpStream, socket_addr: SocketAddr| {
            let new_service = &new_service;
            let shutdown = connection_shutdown.clone();
            async move {
                Ok::<_, io::Error>(
                    accept_tcp_connection(stream, socket_addr, new_service)?
                        .map(|(service, stream)| (service, StopOnShutdown::new(stream, shutdown))),
                )
            }
        };

        let on_process_error = |err: io::Error| {
            tracing::error!("Slave connection error: {err}");
        };

        tracing::info!("Modbus slave server listening on {addr}");
        if let Err(e) = server
            .serve_until(&on_connected, on_process_error, Box::pin(wait_shutdown(shutdown_rx)))
            .await
        {
            tracing::error!("Slave server error: {e}");
        }
        tracing::info!("Slave server stopped");
        let _ = log_tx.send(status_entry("Server stopped".into()));
    });

    Ok((state, log_rx, shutdown_tx))
}

/// RTU unit IDs are 1..=247 (0 is broadcast, 248..=255 are reserved).
#[cfg(feature = "serial")]
fn check_rtu_unit(unit_id: u8) -> anyhow::Result<()> {
    if (1..=247).contains(&unit_id) {
        Ok(())
    } else {
        anyhow::bail!("RTU unit ID must be 1-247, got {unit_id}")
    }
}

#[cfg(feature = "serial")]
fn spawn_rtu_server(
    stream: SerialStream,
    service: SlaveService,
    shutdown_rx: watch::Receiver<bool>,
    log_tx: LogSender,
    started: String,
    stopped: &'static str,
) {
    tokio::spawn(async move {
        let _ = log_tx.send(status_entry(started.clone()));
        let server = tokio_modbus::server::rtu::Server::new(stream);
        tracing::info!("{started}");
        let _ = server.serve_until(service, Box::pin(wait_shutdown(shutdown_rx))).await;
        tracing::info!("{stopped}");
        let _ = log_tx.send(status_entry(stopped.into()));
    });
}

/// Start the slave RTU server on a serial port. Returns `(SlaveState handle, log receiver, shutdown sender)`.
/// Only requests for `unit_id` are answered; broadcast (unit 0) writes are applied silently.
#[cfg(feature = "serial")]
pub async fn start_slave_server_rtu(
    port_name: &str,
    baud_rate: u32,
    data_bits: u8,
    stop_bits: u8,
    parity: RtuParity,
    register_count: u16,
    unit_id: u8,
) -> anyhow::Result<SlaveHandles> {
    check_rtu_unit(unit_id)?;
    let builder = tokio_serial::new(port_name, baud_rate)
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

    let state = new_state(register_count);
    let (log_tx, log_rx) = mpsc::unbounded_channel();
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let service = SlaveService::new(state.clone(), log_tx.clone(), unit_id, Transport::Rtu);

    spawn_rtu_server(
        stream,
        service,
        shutdown_rx,
        log_tx,
        format!("RTU slave (unit {unit_id}) started on {port_name} @ {baud_rate} baud"),
        "RTU server stopped",
    );

    Ok((state, log_rx, shutdown_tx))
}

/// Put a terminal fd into raw mode (8 bits, no translation, no echo, no line editing).
#[cfg(all(feature = "serial", unix))]
fn set_raw(fd: std::os::fd::RawFd) -> io::Result<()> {
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

/// Start the slave RTU server on a virtual serial port (PTY pair).
/// Returns `(SlaveState handle, log receiver, shutdown sender, slave PTY path, slave fd)`.
/// The slave fd must be kept open and used by the in-app master via `dup()`.
#[cfg(all(feature = "serial", unix))]
pub async fn start_slave_server_rtu_virtual(
    register_count: u16,
    unit_id: u8,
) -> anyhow::Result<(
    Arc<Mutex<SlaveState>>,
    mpsc::UnboundedReceiver<SlaveLogEntry>,
    watch::Sender<bool>,
    String,
    std::os::fd::RawFd,
)> {
    use std::ffi::CStr;
    use std::os::unix::io::FromRawFd;

    check_rtu_unit(unit_id)?;

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
        return Err(io::Error::last_os_error().into());
    }

    // openpty leaves the default cooked line discipline: ICRNL turns a 0x0D data byte into 0x0A,
    // ONLCR inserts a 0x0D before every 0x0A, ICANON withholds input until a newline and
    // 0x03/0x11/0x13 act as control characters. RTU frames are binary, so both ends must be raw.
    for fd in [master_fd, slave_fd] {
        if let Err(e) = set_raw(fd) {
            unsafe {
                libc::close(master_fd);
                libc::close(slave_fd);
            }
            return Err(e.into());
        }
    }

    let slave_path = unsafe { CStr::from_ptr(name_buf.as_ptr() as *const libc::c_char) }
        .to_string_lossy()
        .into_owned();

    // Keep slave_fd open — in-app master uses dup() to connect

    // master fd → TTYPort → SerialStream
    let tty_port = unsafe { serialport::TTYPort::from_raw_fd(master_fd) };
    let stream = tokio_serial::SerialStream::try_from(tty_port)?;

    let state = new_state(register_count);
    let (log_tx, log_rx) = mpsc::unbounded_channel();
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let service = SlaveService::new(state.clone(), log_tx.clone(), unit_id, Transport::Rtu);

    spawn_rtu_server(
        stream,
        service,
        shutdown_rx,
        log_tx,
        format!("Virtual RTU slave (unit {unit_id}) started at {slave_path}"),
        "Virtual RTU server stopped",
    );

    Ok((state, log_rx, shutdown_tx, slave_path, slave_fd))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::borrow::Cow;
    use std::time::Duration;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    const UNIT: u8 = 1;

    fn make_service(register_count: usize, unit_id: u8, transport: Transport) -> (Arc<Mutex<SlaveState>>, SlaveService) {
        let state = Arc::new(Mutex::new(SlaveState { registers: vec![0u16; register_count] }));
        let (log_tx, _log_rx) = mpsc::unbounded_channel();
        let service = SlaveService::new(state.clone(), log_tx, unit_id, transport);
        (state, service)
    }

    fn make_tcp_service(register_count: usize) -> (Arc<Mutex<SlaveState>>, SlaveService) {
        make_service(register_count, UNIT, Transport::Tcp)
    }

    /// `Service::call` returns an already-completed `Ready`.
    fn call_slave(
        service: &SlaveService,
        slave: SlaveId,
        request: Request<'static>,
    ) -> Result<Option<Response>, ExceptionCode> {
        service
            .call(SlaveRequest { slave, request })
            .into_inner()
    }

    /// Request addressed to the service's own unit; panics if no response is produced.
    fn call(service: &SlaveService, request: Request<'static>) -> Result<Response, ExceptionCode> {
        call_slave(service, UNIT, request).map(|r| r.expect("a response"))
    }

    #[test]
    fn test_read_holding_registers_basic() {
        let (state, service) = make_tcp_service(10);
        {
            let mut data = state.lock().unwrap();
            data.registers[0] = 100;
            data.registers[1] = 200;
            data.registers[2] = 300;
        }
        match call(&service, Request::ReadHoldingRegisters(0, 3)).unwrap() {
            Response::ReadHoldingRegisters(regs) => assert_eq!(regs, vec![100, 200, 300]),
            _ => panic!("Expected ReadHoldingRegisters response"),
        }
    }

    #[test]
    fn test_write_single_register() {
        let (state, service) = make_tcp_service(10);
        match call(&service, Request::WriteSingleRegister(5, 42)).unwrap() {
            Response::WriteSingleRegister(addr, word) => {
                assert_eq!(addr, 5);
                assert_eq!(word, 42);
            }
            _ => panic!("Expected WriteSingleRegister response"),
        }
        assert_eq!(state.lock().unwrap().registers[5], 42);
    }

    #[test]
    fn test_write_multiple_registers() {
        let (state, service) = make_tcp_service(10);
        match call(&service, Request::WriteMultipleRegisters(0, Cow::Borrowed(&[10, 20, 30]))).unwrap() {
            Response::WriteMultipleRegisters(addr, count) => {
                assert_eq!(addr, 0);
                assert_eq!(count, 3);
            }
            _ => panic!("Expected WriteMultipleRegisters response"),
        }
        assert_eq!(&state.lock().unwrap().registers[..3], &[10, 20, 30]);
    }

    #[test]
    fn test_write_out_of_range_returns_illegal_address() {
        let (_state, service) = make_tcp_service(100);
        assert_eq!(call(&service, Request::WriteSingleRegister(999, 42)), Err(ExceptionCode::IllegalDataAddress));
    }

    #[test]
    fn test_unsupported_function_returns_illegal_function() {
        let (_state, service) = make_tcp_service(10);
        assert_eq!(call(&service, Request::ReadCoils(0, 1)), Err(ExceptionCode::IllegalFunction));
    }

    #[test]
    fn test_read_holding_registers_out_of_range_returns_illegal_address() {
        let (_state, service) = make_tcp_service(5);
        // Modbus requires an address exception, never an empty response.
        assert_eq!(call(&service, Request::ReadHoldingRegisters(10, 3)), Err(ExceptionCode::IllegalDataAddress));
    }

    #[test]
    fn test_read_holding_registers_partial_range() {
        let (_state, service) = make_tcp_service(10);
        // 7..12 with only 10 registers: a partial read must be rejected too.
        assert_eq!(call(&service, Request::ReadHoldingRegisters(7, 5)), Err(ExceptionCode::IllegalDataAddress));
    }

    #[test]
    fn test_write_multiple_registers_out_of_range() {
        let (_state, service) = make_tcp_service(5);
        assert_eq!(
            call(&service, Request::WriteMultipleRegisters(3, Cow::Borrowed(&[10, 20, 30]))),
            Err(ExceptionCode::IllegalDataAddress)
        );
    }

    #[test]
    fn test_write_single_at_last_register() {
        let (state, service) = make_tcp_service(10);
        call(&service, Request::WriteSingleRegister(9, 9999)).unwrap();
        assert_eq!(state.lock().unwrap().registers[9], 9999);
    }

    #[test]
    fn read_quantity_outside_1_to_125_is_illegal_data_value() {
        let (_state, service) = make_tcp_service(300);
        // Modbus Application Protocol v1.1b3 §6.3: quantity of registers 1..=125 (0x7D), else exception 03.
        assert_eq!(call(&service, Request::ReadHoldingRegisters(0, 0)), Err(ExceptionCode::IllegalDataValue));
        assert_eq!(call(&service, Request::ReadHoldingRegisters(0, 126)), Err(ExceptionCode::IllegalDataValue));
        // The quantity check wins over the address check.
        assert_eq!(call(&service, Request::ReadHoldingRegisters(1000, 126)), Err(ExceptionCode::IllegalDataValue));
        assert_eq!(call(&service, Request::ReadHoldingRegisters(1000, 0)), Err(ExceptionCode::IllegalDataValue));
        match call(&service, Request::ReadHoldingRegisters(0, 125)).unwrap() {
            Response::ReadHoldingRegisters(regs) => assert_eq!(regs.len(), 125),
            _ => panic!("Expected ReadHoldingRegisters response"),
        }
    }

    #[test]
    fn write_multiple_quantity_outside_1_to_123_is_illegal_data_value() {
        let (state, service) = make_tcp_service(300);
        // §6.12: quantity of registers 1..=123 (0x7B), else exception 03.
        let too_many = vec![7u16; 124];
        assert_eq!(
            call(&service, Request::WriteMultipleRegisters(0, Cow::Owned(too_many))),
            Err(ExceptionCode::IllegalDataValue)
        );
        assert_eq!(
            call(&service, Request::WriteMultipleRegisters(0, Cow::Owned(Vec::new()))),
            Err(ExceptionCode::IllegalDataValue)
        );
        assert_eq!(state.lock().unwrap().registers[0], 0, "a rejected write must not touch the image");
        let max = vec![7u16; 123];
        assert!(call(&service, Request::WriteMultipleRegisters(0, Cow::Owned(max))).is_ok());
        assert_eq!(state.lock().unwrap().registers[122], 7);
    }

    #[test]
    fn test_read_input_registers_returns_illegal_function() {
        let (_state, service) = make_tcp_service(10);
        assert_eq!(call(&service, Request::ReadInputRegisters(0, 1)), Err(ExceptionCode::IllegalFunction));
    }

    // ── unit-ID filtering ──

    #[test]
    fn rtu_answers_only_its_own_unit() {
        let (state, service) = make_service(10, 5, Transport::Rtu);
        state.lock().unwrap().registers[0] = 77;

        let mine = call_slave(&service, 5, Request::ReadHoldingRegisters(0, 1)).unwrap();
        assert!(matches!(&mine, Some(Response::ReadHoldingRegisters(r)) if r == &[77]));

        // Another unit on the same multi-drop bus: stay silent (no response, no exception).
        assert_eq!(call_slave(&service, 6, Request::ReadHoldingRegisters(0, 1)), Ok(None));
        assert_eq!(call_slave(&service, 247, Request::ReadHoldingRegisters(0, 1)), Ok(None));
        // Not even for requests that would be exceptions.
        assert_eq!(call_slave(&service, 6, Request::ReadHoldingRegisters(500, 1)), Ok(None));
        // and another unit's write must not touch our registers.
        assert_eq!(call_slave(&service, 6, Request::WriteSingleRegister(1, 9)), Ok(None));
        assert_eq!(state.lock().unwrap().registers[1], 0);
    }

    #[test]
    fn rtu_broadcast_writes_apply_but_are_never_answered() {
        let (state, service) = make_service(10, 5, Transport::Rtu);

        assert_eq!(call_slave(&service, 0, Request::WriteSingleRegister(2, 1234)), Ok(None));
        assert_eq!(state.lock().unwrap().registers[2], 1234);

        assert_eq!(
            call_slave(&service, 0, Request::WriteMultipleRegisters(3, Cow::Borrowed(&[1, 2]))),
            Ok(None)
        );
        assert_eq!(&state.lock().unwrap().registers[3..5], &[1, 2]);

        // A broadcast read makes no sense and gets no answer.
        assert_eq!(call_slave(&service, 0, Request::ReadHoldingRegisters(0, 1)), Ok(None));
        // An out-of-range broadcast write is dropped without any exception either.
        assert_eq!(call_slave(&service, 0, Request::WriteSingleRegister(500, 1)), Ok(None));
    }

    #[test]
    fn tcp_rejects_other_units_with_gateway_exception() {
        let (state, service) = make_service(10, 3, Transport::Tcp);
        assert_eq!(
            call_slave(&service, 4, Request::ReadHoldingRegisters(0, 1)),
            Err(ExceptionCode::GatewayTargetDevice)
        );
        assert_eq!(
            call_slave(&service, 4, Request::WriteSingleRegister(0, 5)),
            Err(ExceptionCode::GatewayTargetDevice)
        );
        assert_eq!(state.lock().unwrap().registers[0], 0);
        // Configured unit and the direct-addressing IDs are served.
        for unit in [3, 0xFF, 0x00] {
            assert!(matches!(
                call_slave(&service, unit, Request::ReadHoldingRegisters(0, 1)),
                Ok(Some(Response::ReadHoldingRegisters(_)))
            ));
        }
    }

    // ── over a real TCP socket, with hand-assembled MBAP frames ──

    async fn start_tcp(unit_id: u8) -> (u16, Arc<Mutex<SlaveState>>, watch::Sender<bool>) {
        let port = {
            let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            l.local_addr().unwrap().port()
        };
        let (state, _log, shutdown) = start_slave_server(port, "127.0.0.1", 16, unit_id).await.unwrap();
        (port, state, shutdown)
    }

    async fn exchange(stream: &mut TcpStream, request: &[u8], response_len: usize) -> Vec<u8> {
        stream.write_all(request).await.unwrap();
        let mut buf = vec![0u8; response_len];
        tokio::time::timeout(Duration::from_secs(2), stream.read_exact(&mut buf))
            .await
            .expect("response within 2s")
            .unwrap();
        buf
    }

    #[tokio::test]
    async fn tcp_frames_for_configured_and_foreign_units() {
        let (port, state, _shutdown) = start_tcp(7).await;
        state.lock().unwrap().registers[..2].copy_from_slice(&[0xABCD, 0x0102]);
        let mut s = TcpStream::connect(("127.0.0.1", port)).await.unwrap();

        // Read Holding Registers: transaction 0x0001, protocol 0, length 6, unit 7,
        // FC 03, start address 0x0000, quantity 0x0002.
        let req = [0x00, 0x01, 0x00, 0x00, 0x00, 0x06, 0x07, 0x03, 0x00, 0x00, 0x00, 0x02];
        // Response: same transaction, length 7, unit 7, FC 03, byte count 4, 0xABCD, 0x0102.
        let want = [0x00, 0x01, 0x00, 0x00, 0x00, 0x07, 0x07, 0x03, 0x04, 0xAB, 0xCD, 0x01, 0x02];
        assert_eq!(exchange(&mut s, &req, want.len()).await, want);

        // Same request for unit 9: exception response = FC | 0x80 (0x83), code 0x0B.
        let req = [0x00, 0x02, 0x00, 0x00, 0x00, 0x06, 0x09, 0x03, 0x00, 0x00, 0x00, 0x02];
        let want = [0x00, 0x02, 0x00, 0x00, 0x00, 0x03, 0x09, 0x83, 0x0B];
        assert_eq!(exchange(&mut s, &req, want.len()).await, want);

        // Out-of-range read for our unit: exception 0x02 (Illegal Data Address).
        let req = [0x00, 0x03, 0x00, 0x00, 0x00, 0x06, 0x07, 0x03, 0x00, 0x10, 0x00, 0x01];
        let want = [0x00, 0x03, 0x00, 0x00, 0x00, 0x03, 0x07, 0x83, 0x02];
        assert_eq!(exchange(&mut s, &req, want.len()).await, want);

        // Unit 0xFF (direct addressing) is served.
        let req = [0x00, 0x04, 0x00, 0x00, 0x00, 0x06, 0xFF, 0x03, 0x00, 0x01, 0x00, 0x01];
        let want = [0x00, 0x04, 0x00, 0x00, 0x00, 0x05, 0xFF, 0x03, 0x02, 0x01, 0x02];
        assert_eq!(exchange(&mut s, &req, want.len()).await, want);
    }

    #[tokio::test]
    async fn stopping_the_server_closes_live_connections() {
        let (port, state, shutdown) = start_tcp(1).await;
        let mut s = TcpStream::connect(("127.0.0.1", port)).await.unwrap();

        // Write Single Register 3 = 0x00FF; the response echoes the request.
        let req = [0x00, 0x01, 0x00, 0x00, 0x00, 0x06, 0x01, 0x06, 0x00, 0x03, 0x00, 0xFF];
        assert_eq!(exchange(&mut s, &req, req.len()).await, req);
        assert_eq!(state.lock().unwrap().registers[3], 0xFF);

        shutdown.send(true).unwrap();

        // The server side closes the socket: read() reports EOF (0 bytes) or a reset.
        let mut buf = [0u8; 1];
        let n = tokio::time::timeout(Duration::from_secs(2), s.read(&mut buf))
            .await
            .expect("connection must be closed within 2s of stopping the server");
        assert!(matches!(n, Ok(0) | Err(_)), "unexpected data after stop: {n:?}");

        // ...and a request on a fresh connection is refused as well (listener gone).
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert!(TcpStream::connect(("127.0.0.1", port)).await.is_err());
    }
    /// Request/response pairs printed in Modbus Application Protocol Specification V1.1b3
    /// (§6.3 Read Holding Registers, §6.6 Write Single Register, §6.12 Write Multiple Registers,
    /// §7 exceptions), wrapped in the MBAP header of Modbus Messaging on TCP/IP v1.0b
    /// (transaction 0x1234, protocol 0, unit 0x11 = 17 as in the spec's examples).
    #[tokio::test]
    async fn application_protocol_spec_examples_over_tcp() {
        let port = {
            let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            l.local_addr().unwrap().port()
        };
        let (state, _log, _shutdown) = start_slave_server(port, "127.0.0.1", 200, 0x11).await.unwrap();
        // The §6.3 example reads "registers 108-110" with starting address 0x006B = 107 (the spec
        // numbers registers from 1): they hold 555, 0, 100 = 0x022B, 0x0000, 0x0064.
        state.lock().unwrap().registers[107..110].copy_from_slice(&[0x022B, 0x0000, 0x0064]);
        let mut s = TcpStream::connect(("127.0.0.1", port)).await.unwrap();

        let frame = |pdu: &[u8]| {
            let mut f = vec![0x12, 0x34, 0x00, 0x00, 0x00, pdu.len() as u8 + 1, 0x11];
            f.extend_from_slice(pdu);
            f
        };

        // §6.3: read registers 108..110 -> `03 06 02 2B 00 00 00 64`.
        let want = frame(&[0x03, 0x06, 0x02, 0x2B, 0x00, 0x00, 0x00, 0x64]);
        assert_eq!(exchange(&mut s, &frame(&[0x03, 0x00, 0x6B, 0x00, 0x03]), want.len()).await, want);

        // §6.6: write register 2 = 0x0003; the response echoes the request.
        let echo = frame(&[0x06, 0x00, 0x01, 0x00, 0x03]);
        assert_eq!(exchange(&mut s, &echo, echo.len()).await, echo);
        assert_eq!(state.lock().unwrap().registers[1], 3);

        // §6.12: write registers 2..3 = 0x000A, 0x0102 -> `10 00 01 00 02`.
        let want = frame(&[0x10, 0x00, 0x01, 0x00, 0x02]);
        let req = frame(&[0x10, 0x00, 0x01, 0x00, 0x02, 0x04, 0x00, 0x0A, 0x01, 0x02]);
        assert_eq!(exchange(&mut s, &req, want.len()).await, want);
        assert_eq!(&state.lock().unwrap().registers[1..3], &[0x000A, 0x0102]);

        // §7: an exception response carries the function code with the high bit set and one code byte.
        // Quantity 0 and quantity 126 (> 125) of FC03 are Illegal Data Value (0x03), also when the
        // address is out of range as well.
        for (start, qty) in [([0x00, 0x00], [0x00, 0x00]), ([0x00, 0x00], [0x00, 0x7E]), ([0x27, 0x10], [0x00, 0x7E])] {
            let req = frame(&[0x03, start[0], start[1], qty[0], qty[1]]);
            let want = frame(&[0x83, 0x03]);
            assert_eq!(exchange(&mut s, &req, want.len()).await, want);
        }
        // Address past the image: Illegal Data Address (0x02).
        let want = frame(&[0x83, 0x02]);
        assert_eq!(exchange(&mut s, &frame(&[0x03, 0x27, 0x10, 0x00, 0x01]), want.len()).await, want);
        // A function the device does not implement: Illegal Function (0x01).
        let want = frame(&[0x84, 0x01]);
        assert_eq!(exchange(&mut s, &frame(&[0x04, 0x00, 0x08, 0x00, 0x01]), want.len()).await, want);
    }

    /// Modbus RTU frames (unit, PDU, CRC-16 low byte first) pushed through a pseudo-terminal.
    /// CRCs come from the reference algorithm of Modbus over Serial Line v1.02 Appendix B
    /// (poly 0xA001, init 0xFFFF); `01 03 00 00 00 0A C5 CD` is the classic published example.
    #[cfg(all(feature = "serial", unix))]
    mod rtu_wire {
        use super::*;

        /// Write `request` to the slave end of the PTY and collect whatever comes back within `wait`.
        fn exchange_raw(fd: std::os::fd::RawFd, request: &[u8], wait: Duration) -> Vec<u8> {
            assert_eq!(unsafe { libc::write(fd, request.as_ptr().cast(), request.len()) }, request.len() as isize);
            let deadline = std::time::Instant::now() + wait;
            let mut got = Vec::new();
            loop {
                let left = deadline.saturating_duration_since(std::time::Instant::now());
                if left.is_zero() {
                    return got;
                }
                let mut p = libc::pollfd { fd, events: libc::POLLIN, revents: 0 };
                if unsafe { libc::poll(&mut p, 1, left.as_millis() as libc::c_int) } <= 0 {
                    return got;
                }
                let mut buf = [0u8; 64];
                let n = unsafe { libc::read(fd, buf.as_mut_ptr().cast(), buf.len()) };
                if n <= 0 {
                    return got;
                }
                got.extend_from_slice(&buf[..n as usize]);
                // A short grace period collects the rest of a frame; replies are written in one go.
                std::thread::sleep(Duration::from_millis(20));
            }
        }

        #[tokio::test(flavor = "multi_thread")]
        async fn frames_crc_broadcast_and_foreign_units() {
            let (state, _log, _shutdown, _path, fd) = start_slave_server_rtu_virtual(64, 5).await.unwrap();
            state.lock().unwrap().registers[..2].copy_from_slice(&[0x0102, 0x0304]);
            let quick = Duration::from_millis(150);
            let slow = Duration::from_millis(1500);

            tokio::task::spawn_blocking(move || {
                // Read 2 registers from unit 5 -> `05 03 04 01 02 03 04`.
                let ok = exchange_raw(fd, &[0x05, 0x03, 0x00, 0x00, 0x00, 0x02, 0xC5, 0x8F], slow);
                assert_eq!(ok, [0x05, 0x03, 0x04, 0x01, 0x02, 0x03, 0x04, 0x1E, 0xFC]);

                // Wrong CRC: a slave never answers a corrupted frame.
                assert!(exchange_raw(fd, &[0x05, 0x03, 0x00, 0x00, 0x00, 0x02, 0xC5, 0x8E], quick).is_empty());
                // ... and is back in step for the next valid frame.
                let ok = exchange_raw(fd, &[0x05, 0x03, 0x00, 0x00, 0x00, 0x02, 0xC5, 0x8F], slow);
                assert_eq!(ok.len(), 9, "slave must resynchronise after a corrupted frame: {ok:02X?}");

                // Another unit's request (the classic `01 03 00 00 00 0A C5 CD`): silence.
                assert!(exchange_raw(fd, &[0x01, 0x03, 0x00, 0x00, 0x00, 0x0A, 0xC5, 0xCD], quick).is_empty());

                // Broadcast (unit 0) write register 9 = 0x0D0A: executed, never answered.
                assert!(exchange_raw(fd, &[0x00, 0x06, 0x00, 0x09, 0x0D, 0x0A, 0xDC, 0x8E], quick).is_empty());

                // Exceptions: address past the image -> 02, quantity 0 -> 03.
                let e = exchange_raw(fd, &[0x05, 0x03, 0x00, 0x40, 0x00, 0x01, 0x84, 0x5A], slow);
                assert_eq!(e, [0x05, 0x83, 0x02, 0x81, 0x30]);
                let e = exchange_raw(fd, &[0x05, 0x03, 0x00, 0x00, 0x00, 0x00, 0x44, 0x4E], slow);
                assert_eq!(e, [0x05, 0x83, 0x03, 0x40, 0xF0]);
            })
            .await
            .unwrap();
            assert_eq!(state.lock().unwrap().registers[9], 0x0D0A);
        }
    }
}

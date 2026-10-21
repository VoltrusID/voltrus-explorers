//! Transport adapters that make `tokio-modbus` behave on real wires.
//!
//! `tokio-modbus` matches a reply to its request by reading the *next* frame off the
//! stream. That breaks as soon as one reply is late: the request that timed out is
//! abandoned, its reply shows up during the following request, every later request then
//! reads the reply of the one before it, and the connection stays one frame behind
//! forever. Slow serial gateways trigger this routinely. On RTU it also never waits the
//! inter-frame silence the serial-line specification demands before a master transmits.
//!
//! * [`TxnFilter`] (Modbus/TCP): drops every reply whose MBAP transaction identifier is
//!   not the one of the request in flight, so a late reply can never be mistaken for the
//!   current one.
//! * [`RtuLink`] (serial): holds a request back until the line has been silent for the
//!   t3.5 interval and discards stale input bytes, so the slave sees a clean frame start.

use std::io;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::task::{Context, Poll, ready};

use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};

/// MBAP header: transaction id (2), protocol id (2), length (2), unit id (1).
const MBAP_HEADER_LEN: usize = 7;
/// MBAP length counts the unit id plus the PDU, at most 1 + 253 (Modbus Messaging on TCP/IP v1.0b §3.1.1).
const MBAP_MAX_LENGTH: usize = 254;

/// Modbus/TCP stream that only delivers replies carrying the transaction id of the
/// latest request written to it. See the module docs.
#[derive(Debug)]
pub(crate) struct TxnFilter<T> {
    inner: T,
    /// Transaction id of the request in flight (first two bytes of the frame last written).
    expect: Option<u16>,
    /// Between the first byte of a request and the end of its flush.
    writing: bool,
    /// Bytes read from the socket that do not form a complete frame yet.
    raw: Vec<u8>,
    /// Bytes of accepted frames, not yet handed to the codec.
    ready: Vec<u8>,
    ready_pos: usize,
    /// Number of frames dropped since the counter was last reset.
    dropped: Arc<AtomicUsize>,
}

impl<T> TxnFilter<T> {
    /// Returns the filter and a counter of the frames it dropped.
    pub(crate) fn new(inner: T) -> (Self, Arc<AtomicUsize>) {
        let dropped = Arc::new(AtomicUsize::new(0));
        let filter = Self {
            inner,
            expect: None,
            writing: false,
            raw: Vec::new(),
            ready: Vec::new(),
            ready_pos: 0,
            dropped: dropped.clone(),
        };
        (filter, dropped)
    }

    /// Move every complete frame out of `raw`: matching ones to `ready`, others are dropped.
    fn split_frames(&mut self) {
        loop {
            if self.raw.len() < MBAP_HEADER_LEN {
                return;
            }
            let protocol = u16::from_be_bytes([self.raw[2], self.raw[3]]);
            let length = u16::from_be_bytes([self.raw[4], self.raw[5]]) as usize;
            if protocol != 0 || length < 2 || length > MBAP_MAX_LENGTH {
                // Not a Modbus/TCP frame: no way to find its end, so let the codec report it.
                self.ready.append(&mut self.raw);
                return;
            }
            let total = 6 + length;
            if self.raw.len() < total {
                return;
            }
            let txid = u16::from_be_bytes([self.raw[0], self.raw[1]]);
            if self.expect.is_none_or(|expected| expected == txid) {
                self.ready.extend_from_slice(&self.raw[..total]);
            } else {
                self.dropped.fetch_add(1, Ordering::Relaxed);
                tracing::debug!("Dropped Modbus/TCP reply with transaction id {txid} (waiting for {:?})", self.expect);
            }
            self.raw.drain(..total);
        }
    }
}

impl<T: AsyncRead + Unpin> AsyncRead for TxnFilter<T> {
    fn poll_read(self: Pin<&mut Self>, cx: &mut Context<'_>, out: &mut ReadBuf<'_>) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        loop {
            if this.ready_pos < this.ready.len() {
                let n = (this.ready.len() - this.ready_pos).min(out.remaining());
                out.put_slice(&this.ready[this.ready_pos..this.ready_pos + n]);
                this.ready_pos += n;
                if this.ready_pos == this.ready.len() {
                    this.ready.clear();
                    this.ready_pos = 0;
                }
                return Poll::Ready(Ok(()));
            }
            let mut chunk = [0u8; 512];
            let mut chunk_buf = ReadBuf::new(&mut chunk);
            ready!(Pin::new(&mut this.inner).poll_read(cx, &mut chunk_buf))?;
            let got = chunk_buf.filled();
            if got.is_empty() {
                return Poll::Ready(Ok(())); // EOF
            }
            this.raw.extend_from_slice(got);
            this.split_frames();
        }
    }
}

impl<T: AsyncWrite + Unpin> AsyncWrite for TxnFilter<T> {
    fn poll_write(self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &[u8]) -> Poll<io::Result<usize>> {
        let this = self.get_mut();
        if !this.writing && buf.len() >= 2 {
            this.expect = Some(u16::from_be_bytes([buf[0], buf[1]]));
        }
        this.writing = true;
        Pin::new(&mut this.inner).poll_write(cx, buf)
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        let res = Pin::new(&mut this.inner).poll_flush(cx);
        if res.is_ready() {
            this.writing = false;
        }
        res
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.get_mut().inner).poll_shutdown(cx)
    }
}

/// Silent interval t3.5 between two RTU frames (Modbus over Serial Line v1.02 §2.5.1.1):
/// 3.5 character times, or a fixed 1.75 ms above 19200 baud where the character-based
/// value would be too short for real UARTs and drivers.
#[cfg(feature = "serial")]
pub(crate) fn rtu_silent_interval(baud_rate: u32, data_bits: u8, parity: bool, stop_bits: u8) -> std::time::Duration {
    use std::time::Duration;
    if baud_rate > 19_200 {
        return Duration::from_micros(1_750);
    }
    let bits_per_char = 1 + data_bits as u32 + parity as u32 + stop_bits as u32;
    // 3.5 chars * bits * 1e6 µs / baud, in integer µs with rounding up.
    let micros = (7 * bits_per_char as u64 * 1_000_000).div_ceil(2 * baud_rate.max(1) as u64);
    Duration::from_micros(micros)
}

/// Serial stream that starts a request only after the line was silent for t3.5 and with
/// the receive buffer emptied of stale bytes. See the module docs.
#[cfg(feature = "serial")]
#[derive(Debug)]
pub(crate) struct RtuLink {
    inner: tokio_serial::SerialStream,
    silence: std::time::Duration,
    /// The line counts as silent again at this instant (last byte seen/sent + t3.5).
    quiet_at: tokio::time::Instant,
    /// Between the first byte of a request and the end of its flush.
    writing: bool,
    wait: Option<Pin<Box<tokio::time::Sleep>>>,
}

#[cfg(feature = "serial")]
impl RtuLink {
    pub(crate) fn new(inner: tokio_serial::SerialStream, silence: std::time::Duration) -> Self {
        Self { inner, silence, quiet_at: tokio::time::Instant::now(), writing: false, wait: None }
    }
}

#[cfg(feature = "serial")]
impl AsyncRead for RtuLink {
    fn poll_read(self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &mut ReadBuf<'_>) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        let before = buf.filled().len();
        let res = Pin::new(&mut this.inner).poll_read(cx, buf);
        if matches!(res, Poll::Ready(Ok(()))) && buf.filled().len() > before {
            this.quiet_at = tokio::time::Instant::now() + this.silence;
        }
        res
    }
}

#[cfg(feature = "serial")]
impl AsyncWrite for RtuLink {
    fn poll_write(self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &[u8]) -> Poll<io::Result<usize>> {
        use tokio_serial::SerialPort as _;
        let this = self.get_mut();
        if !this.writing {
            let now = tokio::time::Instant::now();
            if this.quiet_at > now {
                let sleep = this.wait.get_or_insert_with(|| Box::pin(tokio::time::sleep_until(this.quiet_at)));
                ready!(sleep.as_mut().poll(cx));
            }
            this.wait = None;
            // Whatever is still unread is a stale or partial frame; the slave must not
            // see it glued to this request's reply.
            let _ = this.inner.clear(tokio_serial::ClearBuffer::Input);
            this.writing = true;
        }
        Pin::new(&mut this.inner).poll_write(cx, buf)
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        let res = Pin::new(&mut this.inner).poll_flush(cx);
        if matches!(res, Poll::Ready(Ok(()))) && this.writing {
            this.writing = false;
            // `flush` drains the tty, so the last byte just left the wire.
            this.quiet_at = tokio::time::Instant::now() + this.silence;
        }
        res
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.get_mut().inner).poll_shutdown(cx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    /// Read Holding Registers request, transaction 0x0002, unit 1, address 0, quantity 1.
    const REQUEST_2: [u8; 12] = [0x00, 0x02, 0x00, 0x00, 0x00, 0x06, 0x01, 0x03, 0x00, 0x00, 0x00, 0x01];
    /// Matching reply: transaction 0x0002, unit 1, FC 03, 2 bytes: 0x1234.
    const REPLY_2: [u8; 11] = [0x00, 0x02, 0x00, 0x00, 0x00, 0x05, 0x01, 0x03, 0x02, 0x12, 0x34];
    /// Late reply of the abandoned transaction 0x0001: unit 1, FC 03, 2 bytes: 0xBEEF.
    const REPLY_1: [u8; 11] = [0x00, 0x01, 0x00, 0x00, 0x00, 0x05, 0x01, 0x03, 0x02, 0xBE, 0xEF];

    #[tokio::test]
    async fn late_reply_of_an_abandoned_request_is_dropped() {
        let (client_side, mut device) = tokio::io::duplex(256);
        let (mut link, dropped) = TxnFilter::new(client_side);

        link.write_all(&REQUEST_2).await.unwrap();
        link.flush().await.unwrap();
        // The device answers transaction 1 (timed out earlier) before answering transaction 2,
        // in one burst split in the middle of the second frame.
        let mut burst = Vec::new();
        burst.extend_from_slice(&REPLY_1);
        burst.extend_from_slice(&REPLY_2[..5]);
        device.write_all(&burst).await.unwrap();
        device.write_all(&REPLY_2[5..]).await.unwrap();

        let mut got = [0u8; 11];
        link.read_exact(&mut got).await.unwrap();
        assert_eq!(got, REPLY_2);
        assert_eq!(dropped.load(Ordering::Relaxed), 1);
    }

    #[tokio::test]
    async fn filter_follows_the_transaction_id_of_each_new_request() {
        let (client_side, mut device) = tokio::io::duplex(256);
        let (mut link, _dropped) = TxnFilter::new(client_side);

        link.write_all(&REQUEST_2).await.unwrap();
        link.flush().await.unwrap();
        device.write_all(&REPLY_2).await.unwrap();
        let mut got = [0u8; 11];
        link.read_exact(&mut got).await.unwrap();
        assert_eq!(got, REPLY_2);

        // Next request is transaction 1: now REPLY_1 is the matching one.
        let mut request_1 = REQUEST_2;
        request_1[1] = 0x01;
        link.write_all(&request_1).await.unwrap();
        link.flush().await.unwrap();
        device.write_all(&REPLY_1).await.unwrap();
        link.read_exact(&mut got).await.unwrap();
        assert_eq!(got, REPLY_1);
    }

    #[tokio::test]
    async fn non_modbus_bytes_reach_the_codec_unchanged() {
        let (client_side, mut device) = tokio::io::duplex(256);
        let (mut link, dropped) = TxnFilter::new(client_side);
        link.write_all(&REQUEST_2).await.unwrap();
        link.flush().await.unwrap();
        // Protocol id 0x1234 is not Modbus: nothing to filter, the codec reports it.
        let junk = [0x00, 0x02, 0x12, 0x34, 0x00, 0x05, 0x01, 0x03, 0x02, 0x12, 0x34];
        device.write_all(&junk).await.unwrap();
        let mut got = [0u8; 11];
        link.read_exact(&mut got).await.unwrap();
        assert_eq!(got, junk);
        assert_eq!(dropped.load(Ordering::Relaxed), 0);
    }

    #[cfg(feature = "serial")]
    #[test]
    fn silent_interval_follows_the_serial_line_spec() {
        use std::time::Duration;
        // Modbus over Serial Line v1.02 §2.5.1.1: 11-bit characters at 9600 baud => t3.5 = 4.01 ms
        // (4010.4 µs, rounded up).
        assert_eq!(rtu_silent_interval(9600, 8, true, 1), Duration::from_micros(4011));
        // 8N1 is 10 bits per character at 9600 baud: 3.5 * 10 / 9600 = 3.646 ms.
        assert_eq!(rtu_silent_interval(9600, 8, false, 1), Duration::from_micros(3646));
        // 19200 baud, 11 bits: 2.005 ms (2005.2 µs, rounded up).
        assert_eq!(rtu_silent_interval(19_200, 8, true, 1), Duration::from_micros(2006));
        // Above 19200 baud the specification fixes t3.5 at 1.75 ms.
        assert_eq!(rtu_silent_interval(38_400, 8, true, 1), Duration::from_micros(1750));
        assert_eq!(rtu_silent_interval(115_200, 8, false, 1), Duration::from_micros(1750));
        // 1200 baud, 11 bits: 32.08 ms.
        assert_eq!(rtu_silent_interval(1200, 8, true, 1), Duration::from_micros(32_084));
    }
}

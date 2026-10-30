//! Async FOCAS/Ethernet client (TCP 8193).
//!
//! One [`FocasClient`] owns one connection. Calls take `&self` and are serialised internally, so
//! a client can be shared between tasks; each call is one request/response round trip (or several
//! sub-packets of one round trip for [`FocasClient::snapshot`]). A timeout or I/O error drops the
//! connection, because the stream may then hold a late answer; the next call reports
//! [`FocasError::NotConnected`] and the caller reconnects.

use std::sync::Mutex as StdMutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::Mutex;

use crate::capture::{Exchange, Recorder, to_hex};
use crate::codec::{
    CHANNEL_CONTROL, Frame, FrameType, HEADER_LEN, SYNC, SubRequest, SubResponse,
    decode_var_response, encode_var_request,
};
use crate::error::{FocasError, Reading};
use crate::functions::*;
use crate::types::*;

/// TCP port of FOCAS/Ethernet.
pub const DEFAULT_PORT: u16 = 8193;

/// Sub-packets per request. The largest count in the reference samples is nine, so longer
/// multi-requests are split rather than trusting a real controller with more.
pub const MAX_SUBPACKETS: usize = 9;

/// Where and how to connect.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FocasConfig {
    pub host: String,
    pub port: u16,
    /// TCP connect and open handshake (pyfanuc waits 5 s).
    pub connect_timeout_ms: u64,
    /// One request/response round trip.
    pub request_timeout_ms: u64,
}

impl FocasConfig {
    pub fn new(host: impl Into<String>) -> Self {
        FocasConfig {
            host: host.into(),
            ..Default::default()
        }
    }

    pub fn with_port(mut self, port: u16) -> Self {
        self.port = port;
        self
    }
}

impl Default for FocasConfig {
    fn default() -> Self {
        FocasConfig {
            host: "127.0.0.1".into(),
            port: DEFAULT_PORT,
            connect_timeout_ms: 5000,
            request_timeout_ms: 5000,
        }
    }
}

/// A [`FocasClient::connect_with_capture`] that failed, with the frames exchanged before it did.
#[derive(Debug, Clone, PartialEq)]
pub struct CaptureFailure {
    pub error: FocasError,
    pub exchanges: Vec<Exchange>,
}

impl std::fmt::Display for CaptureFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.error.fmt(f)
    }
}

impl std::error::Error for CaptureFailure {}

/// A connection to one CNC.
pub struct FocasClient {
    config: FocasConfig,
    stream: Mutex<Option<TcpStream>>,
    connected: AtomicBool,
    sysinfo: StdMutex<Option<SysInfo>>,
    /// Frame recording, when asked for at connect time (see [`FocasClient::connect_with_capture`]).
    capture: StdMutex<Option<Recorder>>,
}

fn io_err(e: std::io::Error) -> FocasError {
    FocasError::Io(e.to_string())
}

/// Read one whole frame: the header first, then exactly the payload it announces.
pub(crate) async fn read_frame<R: AsyncRead + Unpin>(stream: &mut R) -> Result<Frame, FocasError> {
    read_frame_into(stream, &mut Vec::new()).await
}

/// [`read_frame`] that leaves the bytes it read in `raw`, also when they do not parse as a frame
/// (a capture wants to keep them).
pub(crate) async fn read_frame_into<R: AsyncRead + Unpin>(
    stream: &mut R,
    raw: &mut Vec<u8>,
) -> Result<Frame, FocasError> {
    raw.clear();
    raw.resize(HEADER_LEN, 0);
    stream.read_exact(raw).await.map_err(io_err)?;
    if raw[..4] != SYNC {
        return Err(FocasError::Protocol(
            "frame does not start with a0 a0 a0 a0".into(),
        ));
    }
    let len = usize::from(u16::from_be_bytes([raw[8], raw[9]]));
    raw.resize(HEADER_LEN + len, 0);
    stream
        .read_exact(&mut raw[HEADER_LEN..])
        .await
        .map_err(io_err)?;
    Ok(Frame::decode(raw)?)
}

fn decode<T>(
    call: &'static str,
    response: &SubResponse,
    parse: impl FnOnce(&[u8]) -> Result<T, FocasError>,
) -> Result<T, FocasError> {
    if response.error != 0 {
        return Err(FocasError::Cnc {
            call,
            code: response.error,
        });
    }
    parse(&response.payload)
}

/// Keep the first `n` axes of an axis-ordered list.
fn truncate_to<T>(mut values: Vec<T>, n: Option<usize>) -> Vec<T> {
    if let Some(n) = n {
        values.truncate(n);
    }
    values
}

impl FocasClient {
    /// Connect, open the control stream and read the system info.
    ///
    /// A CNC that answers the open request but refuses `sysinfo` still connects; the
    /// parameter and diagnosis calls then fail until [`FocasClient::sysinfo`] succeeds, because
    /// their payload layout depends on the maximum axis count.
    pub async fn connect(config: FocasConfig) -> Result<Self, FocasError> {
        Self::connect_inner(config, false).await.map_err(|(e, _)| e)
    }

    /// [`FocasClient::connect`] that records every frame exchanged, starting with the open
    /// handshake and the system info read. Retrieve them with [`FocasClient::take_exchanges`]; a
    /// connect that fails hands back what was recorded up to the failure.
    pub async fn connect_with_capture(config: FocasConfig) -> Result<Self, CaptureFailure> {
        Self::connect_inner(config, true)
            .await
            .map_err(|(error, exchanges)| CaptureFailure { error, exchanges })
    }

    async fn connect_inner(
        config: FocasConfig,
        capture: bool,
    ) -> Result<Self, (FocasError, Vec<Exchange>)> {
        let addr = format!("{}:{}", config.host, config.port);
        let timeout = Duration::from_millis(config.connect_timeout_ms);
        let stream = tokio::time::timeout(timeout, TcpStream::connect(&addr))
            .await
            .map_err(|_| {
                (
                    FocasError::ConnectionFailed(format!("{addr}: timed out")),
                    Vec::new(),
                )
            })?
            .map_err(|e| {
                (
                    FocasError::ConnectionFailed(format!("{addr}: {e}")),
                    Vec::new(),
                )
            })?;
        stream.set_nodelay(true).ok();

        let client = FocasClient {
            config,
            stream: Mutex::new(Some(stream)),
            connected: AtomicBool::new(true),
            sysinfo: StdMutex::new(None),
            capture: StdMutex::new(capture.then(Recorder::default)),
        };
        client.set_capture_label("open");
        let open = tokio::time::timeout(
            timeout,
            client.exchange_with(
                &Frame::open_request(CHANNEL_CONTROL),
                Duration::from_millis(client.config.connect_timeout_ms),
            ),
        )
        .await;
        let open = match open {
            Ok(r) => r,
            Err(_) => {
                return Err((
                    FocasError::ConnectionFailed("open request timed out".into()),
                    client.take_exchanges(),
                ));
            }
        };
        let reply = match open {
            Ok(f) => f,
            Err(e) => {
                return Err((
                    FocasError::ConnectionFailed(e.to_string()),
                    client.take_exchanges(),
                ));
            }
        };
        if reply.frame_type != FrameType::OpenResponse {
            client.drop_connection().await;
            return Err((
                FocasError::ConnectionFailed(format!(
                    "expected an open response, got frame type {:04x}",
                    reply.frame_type.code()
                )),
                client.take_exchanges(),
            ));
        }
        client.set_capture_label("sysinfo");
        match client.sysinfo().await {
            Ok(_) => {}
            // The CNC answered but refused or garbled sysinfo: stay connected.
            Err(e @ (FocasError::Cnc { .. } | FocasError::UnexpectedPayload { .. })) => {
                tracing::warn!("FOCAS sysinfo failed after open: {e}");
            }
            Err(e) => {
                return Err((
                    FocasError::ConnectionFailed(e.to_string()),
                    client.take_exchanges(),
                ));
            }
        }
        Ok(client)
    }

    pub fn config(&self) -> &FocasConfig {
        &self.config
    }

    pub fn is_connected(&self) -> bool {
        self.connected.load(Ordering::Relaxed)
    }

    /// The system info read at connect time (or by the last [`FocasClient::sysinfo`]).
    pub fn cached_sysinfo(&self) -> Option<SysInfo> {
        self.sysinfo
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone()
    }

    /// Close the session. The connection is gone afterwards even if the CNC does not answer.
    pub async fn disconnect(&self) -> Result<(), FocasError> {
        let was_connected = self.stream.lock().await.is_some();
        if !was_connected {
            return Ok(());
        }
        let reply = self
            .exchange_with(&Frame::close_request(), Duration::from_millis(1000))
            .await;
        self.drop_connection().await;
        match reply {
            Ok(f) if f.frame_type == FrameType::CloseResponse => Ok(()),
            Ok(f) => Err(FocasError::Protocol(format!(
                "expected a close response, got frame type {:04x}",
                f.frame_type.code()
            ))),
            Err(e) => Err(e),
        }
    }

    async fn drop_connection(&self) {
        *self.stream.lock().await = None;
        self.connected.store(false, Ordering::Relaxed);
    }

    async fn exchange(&self, frame: &Frame) -> Result<Frame, FocasError> {
        self.exchange_with(frame, Duration::from_millis(self.config.request_timeout_ms))
            .await
    }

    async fn exchange_with(&self, frame: &Frame, timeout: Duration) -> Result<Frame, FocasError> {
        let bytes = frame.encode()?;
        let mut guard = self.stream.lock().await;
        let started = Instant::now();
        let mut raw = Vec::new();
        let outcome = {
            let stream = guard.as_mut().ok_or(FocasError::NotConnected)?;
            tokio::time::timeout(timeout, async {
                stream.write_all(&bytes).await.map_err(io_err)?;
                read_frame_into(stream, &mut raw).await
            })
            .await
        };
        // `raw` holds a whole frame after a success, and the bare header after a bad sync; any
        // other failure left it partly unread.
        let (response_complete, error) = match &outcome {
            Ok(Ok(_)) => (true, None),
            Ok(Err(e @ FocasError::Protocol(_))) => (true, Some(e.to_string())),
            Ok(Err(e)) => (false, Some(e.to_string())),
            Err(_) => (
                false,
                Some(format!("no answer within {} ms", timeout.as_millis())),
            ),
        };
        self.record(
            &bytes,
            response_complete.then_some(&raw[..]),
            error,
            started,
        );
        match outcome {
            Ok(Ok(reply)) => Ok(reply),
            Ok(Err(e)) => {
                *guard = None;
                self.connected.store(false, Ordering::Relaxed);
                Err(e)
            }
            Err(_) => {
                *guard = None;
                self.connected.store(false, Ordering::Relaxed);
                Err(FocasError::Timeout(format!(
                    "no answer within {} ms",
                    timeout.as_millis()
                )))
            }
        }
    }

    fn record(
        &self,
        request: &[u8],
        response: Option<&[u8]>,
        error: Option<String>,
        started: Instant,
    ) {
        let mut guard = self.capture.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(rec) = guard.as_mut() {
            rec.exchanges.push(Exchange {
                label: rec.label.clone(),
                request: to_hex(request),
                response: response.map(to_hex),
                error,
                elapsed_ms: u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
            });
        }
    }

    /// Tag the exchanges that follow with `label` (the call they belong to). Does nothing unless
    /// the client was connected with [`FocasClient::connect_with_capture`].
    pub fn set_capture_label(&self, label: &str) {
        if let Some(rec) = self
            .capture
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .as_mut()
        {
            rec.label = label.to_string();
        }
    }

    /// The exchanges recorded since connect (or since the last call), oldest first; recording
    /// goes on. Empty unless the client was connected with [`FocasClient::connect_with_capture`].
    pub fn take_exchanges(&self) -> Vec<Exchange> {
        self.capture
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .as_mut()
            .map(|rec| std::mem::take(&mut rec.exchanges))
            .unwrap_or_default()
    }

    /// Send sub-packets and return one response per request, in order. More than
    /// [`MAX_SUBPACKETS`] are split over several round trips. A non-zero `error` in a response is
    /// left for the caller to judge.
    pub async fn request(&self, subs: &[SubRequest]) -> Result<Vec<SubResponse>, FocasError> {
        if subs.is_empty() {
            return Err(FocasError::InvalidArgument("no sub-packets to send".into()));
        }
        let mut out = Vec::with_capacity(subs.len());
        for chunk in subs.chunks(MAX_SUBPACKETS) {
            let reply = self.exchange(&encode_var_request(chunk)?).await?;
            if reply.frame_type != FrameType::VarResponse {
                return Err(FocasError::Protocol(format!(
                    "expected a data response, got frame type {:04x}",
                    reply.frame_type.code()
                )));
            }
            let responses = decode_var_response(&reply)?;
            if responses.len() != chunk.len() {
                return Err(FocasError::Protocol(format!(
                    "sent {} sub-packets, got {} back",
                    chunk.len(),
                    responses.len()
                )));
            }
            for (req, resp) in chunk.iter().zip(&responses) {
                if req.command != resp.command {
                    return Err(FocasError::Protocol(format!(
                        "response for function {:08x} answers {:08x}",
                        req.command.function, resp.command.function
                    )));
                }
            }
            out.extend(responses);
        }
        Ok(out)
    }

    async fn call(&self, call: &'static str, sub: SubRequest) -> Result<Vec<u8>, FocasError> {
        let response = self
            .request(&[sub])
            .await?
            .into_iter()
            .next()
            .ok_or_else(|| FocasError::Protocol("empty response".into()))?;
        decode(call, &response, |p| Ok(p.to_vec()))
    }

    // ---- identity and status ---------------------------------------------------------------

    /// `cnc_sysinfo`: reads it from the CNC and refreshes the cached copy.
    pub async fn sysinfo(&self) -> Result<SysInfo, FocasError> {
        let payload = self.call("sysinfo", req_sysinfo()).await?;
        let info = parse_sysinfo(&payload)?;
        *self.sysinfo.lock().unwrap_or_else(|p| p.into_inner()) = Some(info.clone());
        Ok(info)
    }

    async fn known_sysinfo(&self) -> Result<SysInfo, FocasError> {
        match self.cached_sysinfo() {
            Some(info) => Ok(info),
            None => self.sysinfo().await,
        }
    }

    /// `cnc_statinfo`: mode, run state, motion, M/S/T/B, emergency, alarm and edit status.
    pub async fn statinfo(&self) -> Result<StatInfo, FocasError> {
        parse_statinfo(&self.call("statinfo", req_statinfo()).await?)
    }

    /// Names of the controlled axes (`X`, `Y`, `Z`, ...), in position order.
    pub async fn axis_names(&self) -> Result<Vec<AxisName>, FocasError> {
        parse_axis_names(&self.call("axis names", req_axis_names()).await?)
    }

    // ---- positions, feed, spindle ----------------------------------------------------------

    /// One kind of position for the controlled axes. Values are scaled by the decimal places the
    /// CNC reports with each number.
    pub async fn read_position(&self, kind: PositionKind) -> Result<Vec<Option<f64>>, FocasError> {
        let responses = self
            .request(&[req_axis_names(), req_position(kind, ALL_AXES)])
            .await?;
        let names = decode("axis names", &responses[0], parse_axis_names)?;
        let values = decode("position", &responses[1], |p| {
            parse_scaled_list("position", p)
        })?;
        Ok(truncate_to(values, Some(names.len())))
    }

    /// Absolute, machine, relative and distance-to-go positions in one round trip.
    pub async fn read_positions(&self) -> Result<AxisPositions, FocasError> {
        let responses = self.request(&position_requests()).await?;
        Ok(build_positions(&responses))
    }

    /// `cnc_actf`: actual feed rate. `None` when the CNC flags the value invalid.
    pub async fn actual_feed(&self) -> Result<Option<f64>, FocasError> {
        parse_scaled_single(
            "actual feed",
            &self.call("actual feed", req_actual_feed()).await?,
        )
    }

    /// `cnc_acts`: actual spindle speed. Some controllers answer `EW_FUNC` (the reference's
    /// samples show one doing so); that surfaces as [`FocasError::Cnc`].
    pub async fn spindle_speed(&self) -> Result<Option<f64>, FocasError> {
        parse_scaled_single(
            "spindle speed",
            &self.call("spindle speed", req_actual_spindle()).await?,
        )
    }

    /// Servo load meter per controlled axis (function `0x56`, as in the reference's multi-packet
    /// sample).
    pub async fn servo_load(&self) -> Result<Vec<Option<f64>>, FocasError> {
        let responses = self.request(&[req_axis_names(), req_servo_load()]).await?;
        let names = decode("axis names", &responses[0], parse_axis_names)?;
        let values = decode("servo load", &responses[1], |p| {
            parse_scaled_list("servo load", p)
        })?;
        Ok(truncate_to(values, Some(names.len())))
    }

    // ---- program and alarms ----------------------------------------------------------------

    /// Running and main program numbers.
    pub async fn program_numbers(&self) -> Result<ProgramNumbers, FocasError> {
        parse_program_numbers(&self.call("program number", req_program_numbers()).await?)
    }

    /// Active sequence (N) number.
    pub async fn sequence_number(&self) -> Result<i32, FocasError> {
        parse_sequence_number(&self.call("sequence number", req_sequence_number()).await?)
    }

    /// Main program name with its path (series 30i and later; others answer an error).
    pub async fn main_program_name(&self) -> Result<String, FocasError> {
        Ok(parse_program_name(
            &self.call("program name", req_program_name()).await?,
        ))
    }

    /// The block text the CNC is executing, up to `chars` characters.
    pub async fn executing_block(&self, chars: u16) -> Result<ExecBlock, FocasError> {
        parse_exec_block(&self.call("executing block", req_exec_block(chars)).await?)
    }

    /// The program directory: number, size and comment of every program.
    pub async fn list_programs(&self) -> Result<Vec<ProgramEntry>, FocasError> {
        let mut out: Vec<ProgramEntry> = Vec::new();
        let mut start = 1u32;
        // The CNC ends the list with an empty payload or an error; the cap only guards a
        // controller that never does.
        for _ in 0..1000 {
            let payload = match self.call("program list", req_program_list(start)).await {
                Ok(p) => p,
                Err(FocasError::Cnc { .. }) if !out.is_empty() => break,
                Err(e) => return Err(e),
            };
            let entries = parse_program_list(&payload)?;
            let Some(last) = entries.last() else { break };
            let next = last.number.saturating_add(1);
            out.extend(entries);
            if next <= start {
                break;
            }
            start = next;
        }
        Ok(out)
    }

    /// `cnc_alarm`: alarm bit field.
    pub async fn alarm_bits(&self) -> Result<AlarmBits, FocasError> {
        parse_alarm_bits(&self.call("alarm", req_alarm_bits()).await?)
    }

    /// `cnc_rdalmmsg`: active alarms with number, type, axis and (optionally) message text.
    pub async fn alarm_messages(&self, query: AlarmQuery) -> Result<Vec<AlarmMessage>, FocasError> {
        if query.max_messages == 0 || query.text_length == 0 {
            return Err(FocasError::InvalidArgument(
                "max_messages and text_length must be at least 1".into(),
            ));
        }
        parse_alarm_messages(
            &query,
            &self
                .call("alarm message", req_alarm_messages(&query))
                .await?,
        )
    }

    // ---- macro variables, parameters, diagnosis, PMC -----------------------------------------

    /// `cnc_rdmacro`: macro variables `first..=last`.
    pub async fn read_macro(&self, first: u32, last: u32) -> Result<Vec<MacroValue>, FocasError> {
        if last < first || u64::from(last - first + 1) * 8 > 60_000 {
            return Err(FocasError::InvalidArgument(format!(
                "macro range {first}..={last} is empty or too long"
            )));
        }
        let payload = self.call("macro read", req_macro_read(first, last)).await?;
        parse_macro_values(first, &payload)
    }

    /// Write a macro variable as a double (function `0xa8`, taken from the pyfocas package; the
    /// reference has no capture of the answer, so success is an all-zero error word).
    pub async fn write_macro(&self, number: u32, value: f64) -> Result<(), FocasError> {
        if number == 0 {
            return Err(FocasError::InvalidArgument("macro number 0".into()));
        }
        if !value.is_finite() {
            return Err(FocasError::InvalidArgument(
                "macro value must be finite".into(),
            ));
        }
        self.call("macro write", req_macro_write(number, value))
            .await
            .map(|_| ())
    }

    async fn read_numbered(
        &self,
        kind: NumberedKind,
        first: u32,
        last: u32,
        axis: i32,
    ) -> Result<Vec<NumberedValues>, FocasError> {
        if last < first {
            return Err(FocasError::InvalidArgument(format!(
                "range {first}..={last} is empty"
            )));
        }
        let info = self.known_sysinfo().await?;
        let name = match kind {
            NumberedKind::Param => "parameter read",
            NumberedKind::Diag => "diagnosis read",
        };
        let payload = self
            .call(name, req_numbered(kind, first, last, axis))
            .await?;
        let axes = info.axis_count().filter(|n| *n > 0).map(usize::from);
        Ok(parse_numbered(kind, usize::from(info.max_axis), &payload)?
            .into_iter()
            .map(|mut v| {
                if v.axis == -1 {
                    v.values = truncate_to(std::mem::take(&mut v.values), axes);
                }
                v
            })
            .collect())
    }

    /// Parameters `first..=last` for `axis` (-1 all axes, 0 for non-axis parameters, else the
    /// 1-based axis). An all-axes read returns one value per controlled axis.
    ///
    /// The payload layout depends on the maximum axis count of [`SysInfo`].
    pub async fn read_params(
        &self,
        first: u32,
        last: u32,
        axis: i32,
    ) -> Result<Vec<NumberedValues>, FocasError> {
        self.read_numbered(NumberedKind::Param, first, last, axis)
            .await
    }

    /// Diagnosis values `first..=last` for `axis`, same conventions as [`Self::read_params`].
    pub async fn read_diag(
        &self,
        first: u32,
        last: u32,
        axis: i32,
    ) -> Result<Vec<NumberedValues>, FocasError> {
        self.read_numbered(NumberedKind::Diag, first, last, axis)
            .await
    }

    /// Read `count` PMC values of `data_type` starting at byte address `first`. Addresses in
    /// the result step by the value size.
    pub async fn read_pmc(
        &self,
        area: PmcArea,
        data_type: PmcDataType,
        first: u32,
        count: u32,
    ) -> Result<Vec<PmcValue>, FocasError> {
        if count == 0 || u64::from(count) * data_type.size() as u64 > 1024 {
            return Err(FocasError::InvalidArgument(
                "PMC reads take 1 to 1024 bytes".into(),
            ));
        }
        let payload = self
            .call("pmc read", req_pmc_read(area, data_type, first, count))
            .await?;
        parse_pmc(data_type, first, &payload)
    }

    // ---- polling ---------------------------------------------------------------------------

    /// Status, positions, feed, spindle, program numbers, sequence number and alarm bits in as
    /// few round trips as [`MAX_SUBPACKETS`] allows. An item the CNC refuses is
    /// [`Reading::Failed`]; the rest is still filled in.
    pub async fn snapshot(&self) -> Result<Snapshot, FocasError> {
        let mut subs = vec![
            req_statinfo(),
            req_alarm_bits(),
            req_program_numbers(),
            req_sequence_number(),
            req_actual_feed(),
            req_actual_spindle(),
        ];
        subs.extend(position_requests());
        let responses = self.request(&subs).await?;
        let r = &responses;
        Ok(Snapshot {
            status: decode("statinfo", &r[0], parse_statinfo).into(),
            alarm_bits: decode("alarm", &r[1], parse_alarm_bits).into(),
            programs: decode("program number", &r[2], parse_program_numbers).into(),
            sequence_number: decode("sequence number", &r[3], parse_sequence_number).into(),
            feed_rate: decode("actual feed", &r[4], |p| {
                parse_scaled_single("actual feed", p)
            })
            .into(),
            spindle_speed: decode("spindle speed", &r[5], |p| {
                parse_scaled_single("spindle speed", p)
            })
            .into(),
            positions: build_positions(&r[6..]),
        })
    }

    /// The optional reads of `query` (servo load, program name, macro variables) in as few
    /// round trips as [`MAX_SUBPACKETS`] allows. Like [`FocasClient::snapshot`], an item the CNC
    /// refuses is [`Reading::Failed`] and the rest is still filled in. Nothing is sent when
    /// `query` asks for nothing.
    pub async fn extended_snapshot(&self, query: &ExtraQuery) -> Result<ExtraSnapshot, FocasError> {
        if let Some(n) = query.macros.iter().find(|n| **n == 0) {
            return Err(FocasError::InvalidArgument(format!("macro number {n}")));
        }
        let mut subs: Vec<SubRequest> = Vec::new();
        if query.servo_load {
            subs.push(req_servo_load());
        }
        if query.program_name {
            subs.push(req_program_name());
        }
        subs.extend(query.macros.iter().map(|n| req_macro_read(*n, *n)));
        if subs.is_empty() {
            return Ok(ExtraSnapshot {
                servo_load: None,
                program_name: None,
                macros: Vec::new(),
            });
        }
        let responses = self.request(&subs).await?;
        let mut it = responses.iter();
        let servo_load = query.servo_load.then(|| {
            let r = it.next().expect("one response per sub-packet");
            decode("servo load", r, |p| parse_scaled_list("servo load", p)).into()
        });
        let program_name = query.program_name.then(|| {
            let r = it.next().expect("one response per sub-packet");
            decode("program name", r, |p| Ok(parse_program_name(p))).into()
        });
        let macros = query
            .macros
            .iter()
            .zip(it)
            .map(|(n, r)| {
                decode("macro read", r, |p| parse_macro_values(*n, p))
                    .and_then(|mut v| {
                        v.pop().ok_or_else(|| FocasError::UnexpectedPayload {
                            call: "macro read",
                            detail: "empty payload".into(),
                        })
                    })
                    .into()
            })
            .collect();
        Ok(ExtraSnapshot {
            servo_load,
            program_name,
            macros,
        })
    }
}

/// Axis names, then absolute, machine, relative and distance-to-go positions.
fn position_requests() -> [SubRequest; 5] {
    [
        req_axis_names(),
        req_position(PositionKind::Absolute, ALL_AXES),
        req_position(PositionKind::Machine, ALL_AXES),
        req_position(PositionKind::Relative, ALL_AXES),
        req_position(PositionKind::DistanceToGo, ALL_AXES),
    ]
}

fn build_positions(r: &[SubResponse]) -> AxisPositions {
    let axes: Reading<Vec<AxisName>> = decode("axis names", &r[0], parse_axis_names).into();
    let count = axes.value().map(Vec::len);
    let list = |resp: &SubResponse| -> Reading<Vec<Option<f64>>> {
        decode("position", resp, |p| parse_scaled_list("position", p))
            .map(|v| truncate_to(v, count))
            .into()
    };
    AxisPositions {
        absolute: list(&r[1]),
        machine: list(&r[2]),
        relative: list(&r[3]),
        distance_to_go: list(&r[4]),
        axes,
    }
}

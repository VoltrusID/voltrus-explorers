//! A read-only probe: every read this crate implements, run once against one controller, with
//! the decoded result of each call kept as JSON.
//!
//! The probe exists to make a hardware check cheap. Connect with
//! [`FocasClient::connect_with_capture`](crate::FocasClient::connect_with_capture), call [`run`],
//! and keep both the [`ProbeReport`] and the captured frames: the report is what the decoder made
//! of them, the capture is the raw evidence. `tests/replay.rs` runs the probe against a capture
//! again and compares it with a stored report, so a capture that was checked against the CNC
//! screen stays a regression test.
//!
//! Nothing here writes to the controller.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::client::FocasClient;
use crate::error::{FocasError, Reading};
use crate::types::*;

/// Value of [`ProbeReport::format`].
pub const REPORT_FORMAT: &str = "focas-probe/1";

/// A parameter or diagnosis number to read, with the axis argument of the call (-1 all axes, 0 a
/// non-axis item, else the 1-based axis).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NumberedProbe {
    pub number: u32,
    pub axis: i32,
}

/// A PMC range to read.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PmcProbe {
    pub area: PmcArea,
    pub data_type: PmcDataType,
    pub address: u32,
    pub count: u32,
}

/// What the probe reads besides the fixed set (identity, status, axes, positions, feed, spindle,
/// servo load, program, sequence, alarms, and the batched `snapshot` / `extended_snapshot`).
///
/// The defaults are values that exist on the simulator; on a real controller some of them are
/// refused (a macro or PMC address the machine builder never used), which shows up in the
/// report as a failed call, not as a probe failure.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProbeOptions {
    /// Macro variables to read, one call each.
    pub macros: Vec<u32>,
    pub params: Vec<NumberedProbe>,
    pub diags: Vec<NumberedProbe>,
    pub pmc: Vec<PmcProbe>,
    /// Also read the program directory (one call per page of entries; long on a full control).
    pub list_programs: bool,
    /// Characters of the executing block to ask for.
    pub exec_block_chars: u16,
}

impl Default for ProbeOptions {
    fn default() -> Self {
        let item = |number, axis| NumberedProbe { number, axis };
        let pmc = |area, data_type, address, count| PmcProbe {
            area,
            data_type,
            address,
            count,
        };
        ProbeOptions {
            macros: vec![100, 500, 501],
            // 1020: axis names (per axis), 3401: a bit parameter, 3741: max spindle speed,
            // 6711 / 6712: part counters.
            params: vec![
                item(1020, -1),
                item(3401, 0),
                item(3741, 0),
                item(6711, 0),
                item(6712, 0),
            ],
            // 308 / 309: servo / pulse coder temperature.
            diags: vec![item(308, -1), item(309, -1)],
            pmc: vec![
                pmc(PmcArea::X, PmcDataType::Byte, 0, 4),
                pmc(PmcArea::Y, PmcDataType::Byte, 0, 4),
                pmc(PmcArea::R, PmcDataType::Byte, 0, 4),
                pmc(PmcArea::D, PmcDataType::Word, 2204, 1),
            ],
            list_programs: false,
            exec_block_chars: 64,
        }
    }
}

/// The outcome of one call.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProbeCall {
    /// What was read (`sysinfo`, `position absolute`, `macro #500`, ...). Also the label of the
    /// frames of this call in the capture.
    pub name: String,
    /// Function number of the request, as in the README table (`01 18`); `-` for the batched
    /// calls that send several.
    pub function: String,
    pub ok: bool,
    /// The decoded result. Present also for a batched call in which some items were refused.
    pub value: Option<Value>,
    pub error: Option<String>,
    /// The `EW_*` code the CNC answered with, when it refused the call.
    pub ew_code: Option<i16>,
}

/// One value an operator can read off the CNC screen.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScreenLine {
    pub item: String,
    pub value: String,
}

/// The result of [`run`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProbeReport {
    /// Always [`REPORT_FORMAT`].
    pub format: String,
    pub host: String,
    pub port: u16,
    pub options: ProbeOptions,
    /// Set when the connection was lost part-way; the remaining calls were not tried.
    pub aborted: Option<String>,
    pub calls: Vec<ProbeCall>,
    /// The values worth comparing with the CNC screen, in readable form.
    pub screen: Vec<ScreenLine>,
}

impl ProbeReport {
    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).expect("a report always serialises")
    }

    pub fn from_json(text: &str) -> Result<ProbeReport, String> {
        let report: ProbeReport = serde_json::from_str(text).map_err(|e| e.to_string())?;
        if report.format != REPORT_FORMAT {
            return Err(format!(
                "report format '{}' is not '{REPORT_FORMAT}'",
                report.format
            ));
        }
        Ok(report)
    }

    /// Number of calls that failed or had a refused item.
    pub fn failed(&self) -> usize {
        self.calls.iter().filter(|c| !c.ok).count()
    }

    /// The report as text for a terminal: what to compare with the screen, then every call.
    pub fn to_text(&self) -> String {
        let mut out = format!("FOCAS probe of {}:{}\n\n", self.host, self.port);
        out.push_str("Compare with the CNC screen\n");
        let width = self.screen.iter().map(|l| l.item.len()).max().unwrap_or(0);
        for l in &self.screen {
            out.push_str(&format!("  {:<width$}  {}\n", l.item, l.value));
        }
        out.push_str("\nEvery call\n");
        for c in &self.calls {
            let state = if c.ok { "ok  " } else { "FAIL" };
            out.push_str(&format!("  {state} {} [{}]", c.name, c.function));
            if let Some(e) = &c.error {
                out.push_str(&format!(": {e}"));
            }
            out.push('\n');
        }
        if let Some(a) = &self.aborted {
            out.push_str(&format!("\nStopped early: {a}\n"));
        }
        out.push_str(&format!(
            "\n{} calls, {} failed\n",
            self.calls.len(),
            self.failed()
        ));
        out
    }
}

/// Decimal places for a position-like number: three when that is exact, else as many as needed.
fn num(v: f64) -> String {
    if (v * 1000.0).round() / 1000.0 == v {
        format!("{v:.3}")
    } else {
        format!("{v}")
    }
}

fn opt_num(v: &Option<f64>) -> String {
    v.map_or_else(|| "invalid".to_string(), num)
}

fn o_number(n: i32) -> String {
    format!("O{n:04}")
}

struct Probe<'a> {
    client: &'a FocasClient,
    calls: Vec<ProbeCall>,
    screen: Vec<ScreenLine>,
    aborted: Option<String>,
}

impl Probe<'_> {
    fn line(&mut self, item: impl Into<String>, value: impl Into<String>) {
        self.screen.push(ScreenLine {
            item: item.into(),
            value: value.into(),
        });
    }

    /// Run one call, keep its outcome, return its value. Once the connection is gone nothing
    /// more is tried.
    async fn step<T: Serialize>(
        &mut self,
        name: &str,
        function: &str,
        call: impl Future<Output = Result<T, FocasError>>,
    ) -> Option<T> {
        if self.aborted.is_some() {
            return None;
        }
        self.client.set_capture_label(name);
        let result = call.await;
        let mut entry = ProbeCall {
            name: name.to_string(),
            function: function.to_string(),
            ok: false,
            value: None,
            error: None,
            ew_code: None,
        };
        let out = match result {
            Ok(v) => {
                entry.ok = true;
                entry.value = Some(serde_json::to_value(&v).unwrap_or(Value::Null));
                Some(v)
            }
            Err(e) => {
                if let FocasError::Cnc { code, .. } = &e {
                    entry.ew_code = Some(*code);
                }
                entry.error = Some(e.to_string());
                if !self.client.is_connected() {
                    self.aborted = Some(format!("connection lost during '{name}': {e}"));
                }
                None
            }
        };
        self.calls.push(entry);
        out
    }

    /// Mark the last call failed when a batched call came back with refused items.
    fn flag_refused(&mut self, refused: Vec<String>) {
        if refused.is_empty() {
            return;
        }
        if let Some(c) = self.calls.last_mut() {
            c.ok = false;
            c.error = Some(format!("refused: {}", refused.join(", ")));
        }
    }
}

fn refused<T>(name: &str, r: &Reading<T>, out: &mut Vec<String>) {
    if let Reading::Failed(f) = r {
        out.push(format!("{name} ({})", f.message));
    }
}

/// Run every read once. The client should have been connected with
/// [`FocasClient::connect_with_capture`] if the raw frames are wanted.
pub async fn run(client: &FocasClient, options: &ProbeOptions) -> ProbeReport {
    let mut p = Probe {
        client,
        calls: Vec::new(),
        screen: Vec::new(),
        aborted: None,
    };

    // ---- identity and status
    if let Some(s) = p.step("sysinfo", "01 18", client.sysinfo()).await {
        p.line(
            "controller",
            format!(
                "series {} {} {}, version {}, {} of max {} axes",
                s.cnc_type,
                s.mt_type,
                s.series,
                s.version,
                s.axes.trim(),
                s.max_axis
            ),
        );
    }
    if let Some(s) = p.step("statinfo", "01 19", client.statinfo()).await {
        p.line("mode", s.aut.label());
        p.line("run state", s.run.label());
        p.line("motion", s.motion.label());
        p.line("M/S/T/B", s.mstb.label());
        p.line("emergency", s.emergency.label());
        p.line("alarm status", s.alarm.label());
        p.line("edit", s.edit.label());
    }

    // ---- axes and positions
    let names: Vec<String> = p
        .step("axis names", "01 89", client.axis_names())
        .await
        .map(|v| v.iter().map(ToString::to_string).collect())
        .unwrap_or_default();
    let mut tables: Vec<(&str, Vec<Option<f64>>)> = Vec::new();
    for (label, kind) in [
        ("absolute", PositionKind::Absolute),
        ("machine", PositionKind::Machine),
        ("relative", PositionKind::Relative),
        ("distance to go", PositionKind::DistanceToGo),
    ] {
        let name = format!("position {label}");
        if let Some(v) = p.step(&name, "01 26", client.read_position(kind)).await {
            tables.push((label, v));
        }
    }
    for (i, axis) in names.iter().enumerate() {
        let parts: Vec<String> = tables
            .iter()
            .filter_map(|(label, v)| v.get(i).map(|x| format!("{label} {}", opt_num(x))))
            .collect();
        p.line(format!("axis {axis}"), parts.join(", "));
    }

    // ---- feed, spindle, servo load
    if let Some(v) = p.step("actual feed", "01 24", client.actual_feed()).await {
        p.line("actual feed", opt_num(&v));
    }
    if let Some(v) = p
        .step("spindle speed", "01 25", client.spindle_speed())
        .await
    {
        p.line("spindle speed", opt_num(&v));
    }
    if let Some(v) = p.step("servo load", "01 56", client.servo_load()).await {
        let parts: Vec<String> = names
            .iter()
            .zip(&v)
            .map(|(n, x)| format!("{n} {}", opt_num(x)))
            .collect();
        p.line("servo load", parts.join(", "));
    }

    // ---- program
    if let Some(v) = p
        .step("program numbers", "01 1c", client.program_numbers())
        .await
    {
        p.line("program running", o_number(v.running));
        p.line("program main", o_number(v.main));
    }
    if let Some(v) = p
        .step("sequence number", "01 1d", client.sequence_number())
        .await
    {
        p.line("sequence number", format!("N{v}"));
    }
    if let Some(v) = p
        .step("program name", "01 b9", client.main_program_name())
        .await
    {
        p.line("program name", v);
    }
    if let Some(v) = p
        .step(
            "executing block",
            "01 20",
            client.executing_block(options.exec_block_chars),
        )
        .await
    {
        p.line("executing block", v.text);
    }
    if options.list_programs
        && let Some(v) = p
            .step("program list", "01 06", client.list_programs())
            .await
    {
        let list: Vec<String> = v.iter().map(|e| o_number(e.number as i32)).collect();
        p.line("programs", format!("{} ({})", list.len(), list.join(" ")));
    }

    // ---- alarms
    if let Some(v) = p.step("alarm bits", "01 1a", client.alarm_bits()).await {
        let types: Vec<&str> = v.types().iter().map(|t| t.label()).collect();
        p.line("alarm bits", format!("{:#010x} ({})", v.0, types.join(" ")));
    }
    if let Some(v) = p
        .step(
            "alarm messages",
            "01 23",
            client.alarm_messages(AlarmQuery::default()),
        )
        .await
    {
        if v.is_empty() {
            p.line("alarms", "none");
        }
        for a in &v {
            p.line(
                format!("alarm {} {}", a.alarm_type.label(), a.number),
                format!(
                    "axis {}: {}",
                    a.axis,
                    a.text.as_deref().unwrap_or("(no text)")
                ),
            );
        }
    }

    // ---- macros, parameters, diagnosis, PMC
    for n in &options.macros {
        let name = format!("macro #{n}");
        if let Some(v) = p.step(&name, "01 15", client.read_macro(*n, *n)).await {
            for m in &v {
                p.line(format!("macro #{}", m.number), opt_num(&m.value));
            }
        }
    }
    for item in &options.params {
        let name = format!("param {} axis {}", item.number, item.axis);
        if let Some(v) = p
            .step(
                &name,
                "01 0e",
                client.read_params(item.number, item.number, item.axis),
            )
            .await
        {
            for n in &v {
                p.line(format!("parameter {}", n.number), join_values(&n.values));
            }
        }
    }
    for item in &options.diags {
        let name = format!("diag {} axis {}", item.number, item.axis);
        if let Some(v) = p
            .step(
                &name,
                "01 30",
                client.read_diag(item.number, item.number, item.axis),
            )
            .await
        {
            for n in &v {
                p.line(format!("diagnosis {}", n.number), join_values(&n.values));
            }
        }
    }
    for item in &options.pmc {
        let name = format!(
            "pmc {}{} {:?} x{}",
            item.area.letter(),
            item.address,
            item.data_type,
            item.count
        );
        if let Some(v) = p
            .step(
                &name,
                "02 8001",
                client.read_pmc(item.area, item.data_type, item.address, item.count),
            )
            .await
        {
            let values: Vec<String> = v
                .iter()
                .map(|x| format!("{}{}={:#x}", item.area.letter(), x.address, x.value))
                .collect();
            p.line(
                format!("pmc {}{}", item.area.letter(), item.address),
                values.join(" "),
            );
        }
    }

    // ---- the batched calls the adapter polls with
    if let Some(s) = p.step("snapshot", "-", client.snapshot()).await {
        let mut bad = Vec::new();
        refused("statinfo", &s.status, &mut bad);
        refused("alarm", &s.alarm_bits, &mut bad);
        refused("program number", &s.programs, &mut bad);
        refused("sequence number", &s.sequence_number, &mut bad);
        refused("feed", &s.feed_rate, &mut bad);
        refused("spindle", &s.spindle_speed, &mut bad);
        refused("axis names", &s.positions.axes, &mut bad);
        refused("absolute", &s.positions.absolute, &mut bad);
        refused("machine", &s.positions.machine, &mut bad);
        refused("relative", &s.positions.relative, &mut bad);
        refused("distance to go", &s.positions.distance_to_go, &mut bad);
        p.flag_refused(bad);
    }
    let extra = ExtraQuery {
        servo_load: true,
        program_name: true,
        macros: options.macros.clone(),
    };
    if let Some(x) = p
        .step("extended snapshot", "-", client.extended_snapshot(&extra))
        .await
    {
        let mut bad = Vec::new();
        if let Some(r) = &x.servo_load {
            refused("servo load", r, &mut bad);
        }
        if let Some(r) = &x.program_name {
            refused("program name", r, &mut bad);
        }
        for (n, r) in extra.macros.iter().zip(&x.macros) {
            refused(&format!("macro #{n}"), r, &mut bad);
        }
        p.flag_refused(bad);
    }

    ProbeReport {
        format: REPORT_FORMAT.into(),
        host: client.config().host.clone(),
        port: client.config().port,
        options: options.clone(),
        aborted: p.aborted,
        calls: p.calls,
        screen: p.screen,
    }
}

fn join_values(values: &[i32]) -> String {
    values
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(", ")
}

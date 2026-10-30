//! "Save capture": a read-only probe of the connected CNC recorded frame by frame, in the files
//! opex-flow's replay harness (`crates/focas-proto/tests/replay.rs`) reads: `NAME.capture.json`
//! (the `focas-capture/1` frames), `NAME.report.json` (the `focas-probe/1` decoded result) and a
//! human summary to check against the CNC screen.
//!
//! The recording is a session of its own (`FocasClient::connect_with_capture`, then
//! `probe::run`, then close), not the frames of the live view: a polling session repeats
//! requests and carries writes and list reads the harness would not ask for again, so it would
//! not replay. Before returning, the capture is replayed through a [`ReplayServer`] and the
//! decoded result compared with the report, the check the harness makes.

use std::collections::BTreeMap;
use std::time::{SystemTime, UNIX_EPOCH};

use focas_proto::probe::{self, ProbeOptions, ProbeReport};
use focas_proto::{Capture, FocasClient, FocasConfig, ReplayServer};
use serde::{Deserialize, Serialize};

/// The most watched macro variables added to the probe's own list.
pub const MAX_WATCHED_MACROS: usize = 8;

/// What the operator says the machine is doing while the capture runs. It goes into the capture's
/// `meta` and is checked against the status the controller reports.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MachineState {
    #[serde(rename = "idle")]
    Idle,
    #[serde(rename = "running")]
    Running,
    #[serde(rename = "feed-hold")]
    FeedHold,
    #[serde(rename = "alarm")]
    Alarm,
    #[serde(rename = "estop")]
    EStop,
    #[serde(rename = "mdi")]
    Mdi,
}

impl MachineState {
    /// The name used in file names and in `meta.machine_state`: the state names of
    /// `docs/focas-field-test.md` (`mill1-idle`, `mill1-feed-hold`, `mill1-estop`, ...).
    pub fn slug(self) -> &'static str {
        match self {
            MachineState::Idle => "idle",
            MachineState::Running => "running",
            MachineState::FeedHold => "feed-hold",
            MachineState::Alarm => "alarm",
            MachineState::EStop => "estop",
            MachineState::Mdi => "mdi",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            MachineState::Idle => "Idle",
            MachineState::Running => "Running",
            MachineState::FeedHold => "Feed hold",
            MachineState::Alarm => "Alarm",
            MachineState::EStop => "E-stop",
            MachineState::Mdi => "MDI",
        }
    }

    /// What the CNC's status line should show in this state, as `(screen item, accepted labels)`.
    fn expects(self) -> (&'static str, &'static [&'static str]) {
        match self {
            MachineState::Idle => ("run state", &["****", "STOP"]),
            MachineState::Running => ("run state", &["STRT"]),
            MachineState::FeedHold => ("run state", &["HOLD"]),
            MachineState::Alarm => ("alarm status", &["ALM", "PMC"]),
            MachineState::EStop => ("emergency", &["EMG"]),
            MachineState::Mdi => ("mode", &["MDI"]),
        }
    }

    /// A note when the controller's status disagrees with the label the operator picked. `None`
    /// when they agree or the status line is missing from the report (the report says why).
    pub fn mismatch(self, report: &ProbeReport) -> Option<String> {
        let (item, accepted) = self.expects();
        let seen = report
            .screen
            .iter()
            .find(|l| l.item == item)
            .map(|l| l.value.as_str())?;
        if accepted.contains(&seen) {
            return None;
        }
        Some(format!(
            "Labelled \"{}\", but the CNC reports {item} {seen} (expected {}). Check the label before sharing the capture.",
            self.label(),
            accepted.join(" or ")
        ))
    }
}

/// Everything the UI needs to save and show a recorded capture.
#[derive(Debug, Clone, Serialize)]
pub struct CaptureBundle {
    /// `NAME.capture.json`, ending in a newline like the repository fixtures.
    pub capture_json: String,
    /// `NAME.report.json`.
    pub report_json: String,
    /// Human summary (`NAME.summary.txt`): what to compare with the CNC screen, every call, notes.
    pub summary: String,
    /// Suggested `NAME`: `<host>-<state>`.
    pub base_name: String,
    pub exchanges: usize,
    pub calls: usize,
    /// Calls the CNC refused or that failed (also in the summary).
    pub failed_calls: usize,
    /// Why the probe stopped early, if it did.
    pub aborted: Option<String>,
    /// `true` when replaying the capture reproduces the report exactly.
    pub replay_ok: bool,
    /// Things worth reading before sharing the files.
    pub notes: Vec<String>,
}

/// The probe options of a capture: the defaults plus the macro variables the UI watches.
pub fn probe_options(watched: &[u32]) -> ProbeOptions {
    let mut options = ProbeOptions::default();
    for n in watched.iter().copied().filter(|n| *n > 0).take(MAX_WATCHED_MACROS) {
        if !options.macros.contains(&n) {
            options.macros.push(n);
        }
    }
    options
}

/// A file-name-safe form of a host name or address.
pub fn sanitize(host: &str) -> String {
    let s: String = host
        .trim()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '.' { c } else { '-' })
        .collect();
    let s = s.trim_matches(['-', '.']).to_string();
    if s.is_empty() { "cnc".into() } else { s }
}

/// `2026-10-30T10:53:36Z` for a Unix time.
pub fn rfc3339_utc(unix_secs: u64) -> String {
    let days = (unix_secs / 86_400) as i64;
    let rem = unix_secs % 86_400;
    // days since 1970-01-01 -> civil date (Howard Hinnant's algorithm)
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z",
        rem / 3_600,
        rem % 3_600 / 60,
        rem % 60
    )
}

fn now_rfc3339() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    rfc3339_utc(secs)
}

/// The privacy note shown before saving and repeated in the summary file.
pub const PRIVACY_NOTE: &str = "A capture holds every request and response frame as hex: the controller's identity, the program \
name and executing block text, macro, parameter and diagnosis values, and the address of the machine. Share it only with the \
machine owner's agreement. Do not edit the hex: a replay test needs the frames intact.";

/// Record one probe session against `config`. `app_version` goes into the capture's `meta.tool`.
pub async fn record(
    config: FocasConfig,
    state: MachineState,
    watched: &[u32],
    app_version: &str,
) -> Result<CaptureBundle, String> {
    let host = config.host.clone();
    let port = config.port;
    let client = FocasClient::connect_with_capture(config)
        .await
        .map_err(|f| {
            format!(
                "cannot open a capture session: {} ({} frame(s) exchanged before the failure)",
                f.error,
                f.exchanges.len()
            )
        })?;
    let options = probe_options(watched);
    let controller = client.cached_sysinfo().map(|s| {
        format!(
            "{} {} series {} version {}",
            s.cnc_type, s.mt_type, s.series, s.version
        )
    });
    let report = probe::run(&client, &options).await;
    client.set_capture_label("close");
    let mut notes = Vec::new();
    if let Err(e) = client.disconnect().await {
        notes.push(format!("Closing the capture session: {e}"));
    }
    let exchanges = client.take_exchanges();

    let mut meta = BTreeMap::from([
        (
            "tool".to_string(),
            format!("CNC Explorer {app_version} (focas-proto)"),
        ),
        ("host".into(), host.clone()),
        ("port".into(), port.to_string()),
        ("probed_at".into(), now_rfc3339()),
        ("os".into(), std::env::consts::OS.into()),
        ("machine_state".into(), state.slug().into()),
    ]);
    if let Some(c) = controller {
        meta.insert("controller".into(), c);
    }
    let capture = Capture::new(meta, exchanges);
    assemble(capture, report, state, &host, notes).await
}

/// Check, summarise and serialise a recorded session.
pub async fn assemble(
    capture: Capture,
    report: ProbeReport,
    state: MachineState,
    host: &str,
    mut notes: Vec<String>,
) -> Result<CaptureBundle, String> {
    for p in capture.problems() {
        notes.push(format!("Malformed frame in the capture: {p}"));
    }
    if let Some(a) = &report.aborted {
        notes.push(format!("The probe stopped early: {a}"));
    }
    if let Some(m) = state.mismatch(&report) {
        notes.push(m);
    }
    let replay_ok = match replays(&capture, &report).await {
        Ok(()) => true,
        Err(e) => {
            notes.push(format!("The capture does not replay to the same report: {e}"));
            false
        }
    };

    let mut summary = format!(
        "CNC Explorer capture of {host}:{}\nMachine state: {}\nRecorded: {}\n{} frame exchange(s), {} call(s), {} failed\n\n",
        report.port,
        state.label(),
        capture.meta.get("probed_at").map_or("-", String::as_str),
        capture.exchanges.len(),
        report.calls.len(),
        report.failed(),
    );
    summary.push_str(&report.to_text());
    summary.push_str("\nNotes\n");
    if notes.is_empty() {
        summary.push_str("  none\n");
    }
    for n in &notes {
        summary.push_str(&format!("  - {n}\n"));
    }
    summary.push_str(&format!(
        "\nPrivacy\n  {PRIVACY_NOTE}\n\nUse\n  Compare the lines under \"Compare with the CNC screen\" with the CNC at the same moment (photograph the screen).\n  \
A capture checked this way becomes a regression test: copy NAME.capture.json and NAME.report.json to\n  \
crates/focas-proto/tests/fixtures/ in opex-flow and run `cargo test -p focas-proto --test replay`.\n"
    ));

    Ok(CaptureBundle {
        base_name: format!("{}-{}", sanitize(host), state.slug()),
        exchanges: capture.exchanges.len(),
        calls: report.calls.len(),
        failed_calls: report.failed(),
        aborted: report.aborted.clone(),
        replay_ok,
        capture_json: capture.to_json() + "\n",
        report_json: report.to_json() + "\n",
        summary,
        notes,
    })
}

/// What `tests/replay.rs` of focas-proto does with a fixture: probe a [`ReplayServer`] serving the
/// capture with the report's options and require the same decoded result, every request found and
/// every recorded exchange asked for.
pub async fn replays(capture: &Capture, report: &ProbeReport) -> Result<(), String> {
    let server = ReplayServer::start(capture).await?;
    let config = FocasConfig::new("127.0.0.1").with_port(server.port());
    let outcome = async {
        let client = FocasClient::connect(config)
            .await
            .map_err(|e| format!("connect to the replay: {e}"))?;
        let again = probe::run(&client, &report.options).await;
        let _ = client.disconnect().await;
        Ok::<_, String>(again)
    }
    .await;
    let (misses, unplayed) = (server.misses(), server.unplayed());
    server.stop().await;
    let again = outcome?;
    if !misses.is_empty() {
        return Err(format!("{} request(s) missing from the capture", misses.len()));
    }
    if unplayed != 0 {
        return Err(format!("{unplayed} recorded exchange(s) were not asked for"));
    }
    let same = again.options == report.options
        && again.aborted == report.aborted
        && again.calls == report.calls
        && again.screen == report.screen;
    if same {
        Ok(())
    } else {
        Err("the decoded result differs".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rfc3339_matches_known_dates() {
        assert_eq!(rfc3339_utc(0), "1970-01-01T00:00:00Z");
        assert_eq!(rfc3339_utc(951_782_400), "2000-02-29T00:00:00Z");
        assert_eq!(rfc3339_utc(1_782_816_816), "2026-06-30T10:53:36Z");
    }

    #[test]
    fn file_names_are_safe() {
        assert_eq!(sanitize("192.168.1.50"), "192.168.1.50");
        assert_eq!(sanitize(" mill 1/../x "), "mill-1-..-x");
        assert_eq!(sanitize("///"), "cnc");
    }

    #[test]
    fn watched_macros_extend_the_probe_without_duplicates() {
        let base = ProbeOptions::default().macros;
        assert_eq!(probe_options(&[]).macros, base);
        let m = probe_options(&[500, 700, 0, 700]).macros;
        assert_eq!(m, [base.clone(), vec![700]].concat());
        let many: Vec<u32> = (1000..1100).collect();
        assert_eq!(probe_options(&many).macros.len(), base.len() + MAX_WATCHED_MACROS);
    }

    #[test]
    fn machine_state_serialises_as_its_slug() {
        for s in [
            MachineState::Idle,
            MachineState::Running,
            MachineState::FeedHold,
            MachineState::Alarm,
            MachineState::EStop,
            MachineState::Mdi,
        ] {
            assert_eq!(serde_json::to_string(&s).unwrap(), format!("\"{}\"", s.slug()));
        }
    }
}

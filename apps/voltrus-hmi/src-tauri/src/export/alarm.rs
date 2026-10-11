use crate::*;
use super::{js_escape, json_for_script};

/// Height of the always-visible alarm header strip. The export reserves this
/// slot above the screens whenever alarms are configured, so raising,
/// clearing or toggling alarms never moves controls under the operator.
pub const ALARM_HEADER_PX: u32 = 28;

/// Generate alarm banner HTML (fixed position below the nav bar, or at the
/// top edge when there is none — `top` is the screens' top offset). The
/// header strip is always shown; the alarm list drops down over the screen
/// as an overlay.
pub fn render_alarm_banner_html(top: u32) -> String {
    // The list never opens by itself (an overlay appearing under a finger can
    // turn a tap into an unintended Ack); new alarms pulse the header instead.
    format!(r#"<style>
@keyframes alarmPulseCritical{{0%,100%{{background:#111}}50%{{background:#7f1d1d}}}}
@keyframes alarmPulseWarning{{0%,100%{{background:#111}}50%{{background:#78350f}}}}
#alarm-header.pulse-critical{{animation:alarmPulseCritical 1s ease-in-out infinite}}
#alarm-header.pulse-warning{{animation:alarmPulseWarning 1s ease-in-out infinite}}
</style>
<div id="alarm-banner" style="position:fixed;top:{top}px;left:0;right:0;z-index:99">
    <div id="alarm-header" onclick="toggleAlarmBanner()" role="button" aria-label="Show or hide the alarm list" style="display:flex;align-items:center;justify-content:space-between;height:{h}px;box-sizing:border-box;padding:0 12px;background:#111;border-bottom:1px solid #333;user-select:none">
        <span style="font-size:12px;color:#666;font-weight:600" id="alarm-count">No active alarms</span>
        <span style="display:none;color:#999;font-size:14px" id="alarm-toggle">&#x25BC;</span>
    </div>
    <div id="alarm-list" style="display:none;max-height:120px;overflow-y:auto;padding:4px 12px;background:#1a1a2e;border-bottom:2px solid #333;box-shadow:0 6px 16px rgba(0,0,0,0.5)"></div>
</div>"#, top = top, h = ALARM_HEADER_PX)
}

/// Validate a SCADA base URL: http(s) scheme, conservative charset.
fn safe_scada_url(url: &str) -> &str {
    let valid = (url.starts_with("http://") || url.starts_with("https://"))
        && !url.contains('"')
        && !url.contains('\'')
        && !url.contains('<')
        && !url.contains('>')
        && !url.contains('\\');
    if valid {
        url
    } else {
        "http://localhost"
    }
}

/// Generate alarm banner JS — SCADA polling + PLC variable monitoring + acknowledge.
pub fn render_alarm_banner_js(alarm_rules: &[AlarmRule], scada_url: &str) -> String {
    // Rule values are free-form user strings; escape so they cannot break out
    // of the embedded JSON (and thus the <script> block).
    let rules_json = json_for_script(&serde_json::to_string(alarm_rules).unwrap_or_else(|_| "[]".to_string()));
    let scada_url = safe_scada_url(scada_url);
    let scada_url_js = js_escape(scada_url);
    let has_scada = scada_url != "http://localhost";

    let scada_poll_js = if has_scada {
        format!(
            r#"
var scadaAlarms = [];
function connectSCADAAlarms() {{
    fetch("{scada_url_js}/api/v1/alarms")
        .then(function(r) {{ return r.json(); }})
        .then(function(data) {{
            scadaAlarms = (data.alarms || []).filter(function(a) {{ return !a.acknowledged; }});
            renderAlarms();
        }})
        .catch(function() {{}});
}}
function acknowledgeSCADAAlarm(id) {{
    fetch("{scada_url_js}/api/v1/alarms/" + id + "/acknowledge", {{ method: 'POST' }})
        .then(function() {{ connectSCADAAlarms(); }})
        .catch(function() {{}});
}}
"#,
            scada_url_js = scada_url_js,
        )
    } else {
        "var scadaAlarms = [];\n".to_string()
    };

    format!(
        r#"
// --- Alarm Banner ---
var alarmRules = {rules_json};
var plcAlarms = {{}};
var acknowledgedPlcAlarms = {{}};
{scada_poll_js}

// Runs on every PLC frame: only edge changes touch alarm state, so the
// raise timestamp is kept and the list is not rebuilt per frame.
function monitorPLCAlarms(varName, value) {{
    var changed = false;
    for (var i = 0; i < alarmRules.length; i++) {{
        var rule = alarmRules[i];
        if (rule.source !== 'plc' || rule.variableName !== varName) continue;
        if (value === true || value === 1) {{
            if (!acknowledgedPlcAlarms[rule.id] && !plcAlarms[rule.id]) {{
                plcAlarms[rule.id] = {{ id: rule.id, message: rule.message, severity: rule.severity, timestamp: new Date().toISOString() }};
                changed = true;
            }}
        }} else if (plcAlarms[rule.id] || acknowledgedPlcAlarms[rule.id]) {{
            delete plcAlarms[rule.id];
            delete acknowledgedPlcAlarms[rule.id];
            changed = true;
        }}
    }}
    if (changed) renderAlarms();
}}

function acknowledgeAlarm(alarmId) {{
    for (var i = 0; i < scadaAlarms.length; i++) {{
        if (scadaAlarms[i].id == alarmId) {{ acknowledgeSCADAAlarm(alarmId); return; }}
    }}
    acknowledgedPlcAlarms[alarmId] = true;
    delete plcAlarms[alarmId];
    renderAlarms();
}}

var alarmListKey = null;
var alarmListOpen = false;
var alarmListOpenedAt = 0;
// Ack buttons are armed only this long after the operator opens the list, so
// the tap that opened it (or a stray double tap) cannot acknowledge an alarm.
var ACK_ARM_MS = 500;

function setAlarmPulse(severity) {{
    var header = document.getElementById('alarm-header');
    if (!header) return;
    header.className = severity ? 'pulse-' + severity : '';
}}

function highestSeverity(alarms) {{
    for (var i = 0; i < alarms.length; i++) if (alarms[i].severity === 'critical') return 'critical';
    return alarms.length ? 'warning' : '';
}}

// Alarm times: local wall-clock time; non-date values pass through unchanged.
function formatAlarmTime(ts) {{
    if (!ts) return '';
    var d = new Date(ts);
    return isNaN(d.getTime()) ? String(ts) : d.toLocaleTimeString();
}}

function renderAlarms() {{
    var allAlarms = scadaAlarms.slice();
    for (var key in plcAlarms) {{
        if (plcAlarms.hasOwnProperty(key)) allAlarms.push(plcAlarms[key]);
    }}
    var list = document.getElementById('alarm-list');
    var count = document.getElementById('alarm-count');
    var toggle = document.getElementById('alarm-toggle');
    if (!list || !count || !toggle) return;

    // Rebuild only when the set of alarms changes (SCADA polls re-render too),
    // so Ack taps are not lost to a DOM rebuild mid-press.
    var key = allAlarms.map(function(a) {{ return a.id + '|' + a.severity + '|' + a.message + '|' + a.timestamp; }}).join('\n');
    if (key === alarmListKey) return;
    var prevIds = alarmListKey === null ? [] : alarmListKey.split('\n').map(function(k) {{ return k.split('|')[0]; }});
    alarmListKey = key;

    if (allAlarms.length === 0) {{
        count.textContent = 'No active alarms';
        count.style.color = '#666';
        toggle.style.display = 'none';
        alarmListOpen = false;
        list.style.display = 'none';
        setAlarmPulse('');
        while (list.firstChild) list.removeChild(list.firstChild);
        return;
    }}
    var top = highestSeverity(allAlarms);
    count.textContent = allAlarms.length + ' Active Alarm' + (allAlarms.length > 1 ? 's' : '');
    count.style.color = top === 'critical' ? '#ef4444' : '#f59e0b';
    toggle.style.display = 'inline';
    // Never auto-open: a newly raised alarm only pulses the header (in the
    // highest active severity) until the operator opens the list.
    if (!alarmListOpen) {{
        for (var n = 0; n < allAlarms.length; n++) {{
            if (prevIds.indexOf(String(allAlarms[n].id)) === -1) {{ setAlarmPulse(top); break; }}
        }}
    }}

    // Rows may shift under the operator's finger when an open list is rebuilt
    // (e.g. a SCADA alarm lands above the PLC rows): re-arm the Ack guard.
    if (alarmListOpen) alarmListOpenedAt = Date.now();

    // Clear existing children
    while (list.firstChild) list.removeChild(list.firstChild);

    for (var i = 0; i < allAlarms.length; i++) {{
        var a = allAlarms[i];
        var row = document.createElement('div');
        row.style.cssText = 'display:flex;align-items:center;justify-content:space-between;padding:4px 8px;margin:2px 0;border-left:3px solid;border-radius:4px';
        row.style.background = a.severity === 'critical' ? '#7f1d1d' : '#78350f';
        row.style.borderColor = a.severity === 'critical' ? '#dc2626' : '#f59e0b';

        var msg = document.createElement('span');
        msg.style.cssText = 'font-size:12px;color:#eee;flex:1;min-width:0';
        msg.textContent = (typeof t === 'function') ? t(a.message || '') : (a.message || '');

        var ts = document.createElement('span');
        ts.style.cssText = 'font-size:10px;color:#999;margin:0 8px';
        ts.textContent = formatAlarmTime(a.timestamp);

        var btn = document.createElement('button');
        btn.style.cssText = 'background:#333;color:#ccc;border:1px solid #555;border-radius:3px;padding:2px 8px;font-size:10px;cursor:pointer';
        btn.textContent = 'Ack';
        (function(aid) {{
            btn.onclick = function(ev) {{
                if (ev) ev.stopPropagation();
                if (Date.now() - alarmListOpenedAt < ACK_ARM_MS) return; // not armed yet
                acknowledgeAlarm(aid);
            }};
        }})(a.id);

        row.appendChild(msg);
        row.appendChild(ts);
        row.appendChild(btn);
        list.appendChild(row);
    }}
}}

function toggleAlarmBanner() {{
    var list = document.getElementById('alarm-list');
    var btn = document.getElementById('alarm-toggle');
    if (!list || !btn || !list.firstChild) return; // nothing to show
    alarmListOpen = !alarmListOpen;
    if (alarmListOpen) {{
        alarmListOpenedAt = Date.now();
        setAlarmPulse(''); // seen
    }}
    list.style.display = alarmListOpen ? 'block' : 'none';
    btn.innerHTML = alarmListOpen ? '&#x25B2;' : '&#x25BC;';
}}

// Hook into existing bindings for PLC alarm monitoring
(function() {{
    for (var i = 0; i < alarmRules.length; i++) {{
        var rule = alarmRules[i];
        if (rule.source !== 'plc' || !rule.variableName) continue;
        bindings[rule.variableName] = bindings[rule.variableName] || [];
        (function(rn) {{
            bindings[rn].push(function(v) {{ monitorPLCAlarms(rn, v); }});
        }})(rule.variableName);
    }}
}})();

{scada_start}"#,
        rules_json = rules_json,
        scada_poll_js = scada_poll_js,
        scada_start = if has_scada { "setInterval(connectSCADAAlarms, 5000); connectSCADAAlarms();" } else { "" },
    )
}

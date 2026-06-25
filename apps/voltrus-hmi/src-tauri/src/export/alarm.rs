use crate::*;

/// Generate alarm banner HTML (fixed position below nav bar).
pub fn render_alarm_banner_html() -> String {
    r#"<div id="alarm-banner" style="display:none;position:fixed;top:40px;left:0;right:0;z-index:99;max-height:120px;overflow-y:auto;background:#1a1a2e;border-bottom:2px solid #333">
    <div style="display:flex;align-items:center;justify-content:space-between;padding:4px 12px;background:#111">
        <span style="font-size:12px;color:#ef4444;font-weight:600" id="alarm-count">0 Active Alarms</span>
        <button onclick="toggleAlarmBanner()" style="background:none;border:none;color:#999;cursor:pointer;font-size:16px" id="alarm-toggle">&#x25BC;</button>
    </div>
    <div id="alarm-list" style="padding:4px 12px"></div>
</div>"#.to_string()
}

/// Generate alarm banner JS — SCADA polling + PLC variable monitoring + acknowledge.
pub fn render_alarm_banner_js(alarm_rules: &[AlarmRule], scada_url: &str) -> String {
    let rules_json = serde_json::to_string(alarm_rules).unwrap_or_else(|_| "[]".to_string());
    let has_scada = !scada_url.is_empty();

    let scada_poll_js = if has_scada {
        format!(
            r#"
var scadaAlarms = [];
function connectSCADAAlarms() {{
    fetch("{scada_url}/api/v1/alarms")
        .then(function(r) {{ return r.json(); }})
        .then(function(data) {{
            scadaAlarms = (data.alarms || []).filter(function(a) {{ return !a.acknowledged; }});
            renderAlarms();
        }})
        .catch(function() {{}});
}}
function acknowledgeSCADAAlarm(id) {{
    fetch("{scada_url}/api/v1/alarms/" + id + "/acknowledge", {{ method: 'POST' }})
        .then(function() {{ connectSCADAAlarms(); }})
        .catch(function() {{}});
}}
"#
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

function monitorPLCAlarms(varName, value) {{
    for (var i = 0; i < alarmRules.length; i++) {{
        var rule = alarmRules[i];
        if (rule.source !== 'plc' || rule.variableName !== varName) continue;
        if (value === true || value === 1) {{
            if (!acknowledgedPlcAlarms[rule.id]) {{
                plcAlarms[rule.id] = {{ id: rule.id, message: rule.message, severity: rule.severity, timestamp: new Date().toISOString() }};
            }}
        }} else {{
            delete plcAlarms[rule.id];
            delete acknowledgedPlcAlarms[rule.id];
        }}
    }}
    renderAlarms();
}}

function acknowledgeAlarm(alarmId) {{
    for (var i = 0; i < scadaAlarms.length; i++) {{
        if (scadaAlarms[i].id == alarmId) {{ acknowledgeSCADAAlarm(alarmId); return; }}
    }}
    acknowledgedPlcAlarms[alarmId] = true;
    delete plcAlarms[alarmId];
    renderAlarms();
}}

function renderAlarms() {{
    var allAlarms = scadaAlarms.slice();
    for (var key in plcAlarms) {{
        if (plcAlarms.hasOwnProperty(key)) allAlarms.push(plcAlarms[key]);
    }}
    var banner = document.getElementById('alarm-banner');
    var list = document.getElementById('alarm-list');
    var count = document.getElementById('alarm-count');
    if (!banner || !list || !count) return;

    if (allAlarms.length === 0) {{ banner.style.display = 'none'; return; }}
    banner.style.display = 'block';
    count.textContent = allAlarms.length + ' Active Alarm' + (allAlarms.length > 1 ? 's' : '');

    // Clear existing children
    while (list.firstChild) list.removeChild(list.firstChild);

    for (var i = 0; i < allAlarms.length; i++) {{
        var a = allAlarms[i];
        var row = document.createElement('div');
        row.style.cssText = 'display:flex;align-items:center;justify-content:space-between;padding:4px 8px;margin:2px 0;border-left:3px solid;border-radius:4px';
        row.style.background = a.severity === 'critical' ? '#7f1d1d' : '#78350f';
        row.style.borderColor = a.severity === 'critical' ? '#dc2626' : '#f59e0b';

        var msg = document.createElement('span');
        msg.style.cssText = 'font-size:12px;color:#eee';
        msg.textContent = a.message || '';

        var ts = document.createElement('span');
        ts.style.cssText = 'font-size:10px;color:#999;margin:0 8px';
        ts.textContent = a.timestamp || '';

        var btn = document.createElement('button');
        btn.style.cssText = 'background:#333;color:#ccc;border:1px solid #555;border-radius:3px;padding:2px 8px;font-size:10px;cursor:pointer';
        btn.textContent = 'Ack';
        (function(aid) {{ btn.onclick = function() {{ acknowledgeAlarm(aid); }}; }})(a.id);

        row.appendChild(msg);
        row.appendChild(ts);
        row.appendChild(btn);
        list.appendChild(row);
    }}
}}

function toggleAlarmBanner() {{
    var list = document.getElementById('alarm-list');
    var btn = document.getElementById('alarm-toggle');
    if (!list || !btn) return;
    if (list.style.display === 'none') {{
        list.style.display = 'block'; btn.innerHTML = '&#x25BC;';
    }} else {{
        list.style.display = 'none'; btn.innerHTML = '&#x25B2;';
    }}
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

{scada_start}
"#,
        rules_json = rules_json,
        scada_poll_js = scada_poll_js,
        scada_start = if has_scada { "setInterval(connectSCADAAlarms, 5000); connectSCADAAlarms();" } else { "" },
    )
}

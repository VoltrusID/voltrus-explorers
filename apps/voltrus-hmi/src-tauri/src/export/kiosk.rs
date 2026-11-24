/// Generate kiosk mode JS — fullscreen, wake lock, watchdog overlay.
/// The watchdog is reset from the WS runtime's own onopen/onmessage handlers
/// (they call `resetWatchdog()` when present), so it keeps working across
/// reconnects — the WS object is recreated on every connectWS() call.
pub fn render_kiosk_js() -> String {
    r#"
// --- Kiosk Mode ---
var watchdogTimer = null;
var watchdogTimeout = 30000; // 30s

function goFullscreen() {
    var el = document.documentElement;
    if (el.requestFullscreen) el.requestFullscreen();
    else if (el.webkitRequestFullscreen) el.webkitRequestFullscreen();
}

function requestWakeLock() {
    if ('wakeLock' in navigator) {
        navigator.wakeLock.request('screen').catch(function() {});
    }
}

function showConnectionLost() {
    var overlay = document.getElementById('connection-lost');
    if (overlay) overlay.style.display = 'flex';
}

function hideConnectionLost() {
    var overlay = document.getElementById('connection-lost');
    if (overlay) overlay.style.display = 'none';
}

function resetWatchdog() {
    if (watchdogTimer) clearTimeout(watchdogTimer);
    watchdogTimer = setTimeout(showConnectionLost, watchdogTimeout);
}

// Start watchdog; the WS runtime keeps resetting it while data flows.
resetWatchdog();

// Request wake lock on load
requestWakeLock();
"#
    .to_string()
}

/// Generate kiosk overlay HTML for connection lost state.
pub fn render_kiosk_overlay_html() -> String {
    r#"<div id="connection-lost" style="display:none;position:fixed;inset:0;z-index:300;background:rgba(0,0,0,0.85);align-items:center;justify-content:center;flex-direction:column">
    <div style="font-size:48px;color:#ef4444;margin-bottom:16px">⚠</div>
    <div style="font-size:20px;color:#ccc;margin-bottom:8px">Connection Lost</div>
    <div style="font-size:12px;color:#666;margin-bottom:24px">Check PLC connection and network</div>
    <button onclick="hideConnectionLost();connectWS();" style="background:#2a2a3a;color:#ccc;border:1px solid #444;border-radius:6px;padding:8px 24px;cursor:pointer;font-size:14px">Reconnect</button>
</div>"#.to_string()
}

/// Generate fullscreen button HTML for status bar.
pub fn render_fullscreen_button_html() -> String {
    r#"<button onclick="goFullscreen()" style="background:none;border:none;color:#666;cursor:pointer;font-size:11px" title="Fullscreen">⛶</button>"#.to_string()
}

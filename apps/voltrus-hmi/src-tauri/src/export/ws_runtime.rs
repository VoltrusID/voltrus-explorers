use super::js_escape;

/// Generate the WebSocket runtime JS: screen navigation, binary parser, force-var, button handler.
pub fn render_ws_runtime_js(plc_ip: &str) -> String {
    let plc_ip_js = js_escape(plc_ip);
    format!(
        r#"// --- Screen Navigation ---
function switchScreen(idx) {{
    var screens = document.querySelectorAll('.screen');
    for (var i = 0; i < screens.length; i++) {{
        if (i === idx) {{
            screens[i].classList.add('active');
        }} else {{
            screens[i].classList.remove('active');
        }}
    }}
    var btns = document.querySelectorAll('.nav-btn');
    for (var i = 0; i < btns.length; i++) {{
        btns[i].classList.toggle('active', i === idx);
    }}
    currentScreen = idx;
}}

// --- WebSocket Connection ---
var ws = null;
var lastUpdate = Date.now();
var reconnectDelay = 1000;
var maxReconnectDelay = 30000;
var reconnectTimer = null;

function scheduleReconnect() {{
    if (reconnectTimer) clearTimeout(reconnectTimer);
    reconnectTimer = setTimeout(function() {{
        reconnectTimer = null;
        connectWS();
    }}, reconnectDelay);
    reconnectDelay = Math.min(reconnectDelay * 2, maxReconnectDelay);
}}

function connectWS() {{
    if (ws && (ws.readyState === WebSocket.OPEN || ws.readyState === WebSocket.CONNECTING)) return;
    try {{
        ws = new WebSocket('ws://{plc_ip_js}/ws/plc');
        ws.binaryType = 'arraybuffer';

        ws.onopen = function() {{
            document.getElementById('wsStatus').className = 'status-dot connected';
            document.getElementById('wsLabel').textContent = 'Connected';
            reconnectDelay = 1000;
            if (typeof hideConnectionLost === 'function') hideConnectionLost();
            if (typeof resetWatchdog === 'function') resetWatchdog();
        }};

        ws.onclose = function() {{
            document.getElementById('wsStatus').className = 'status-dot disconnected';
            document.getElementById('wsLabel').textContent = 'Disconnected';
            scheduleReconnect();
        }};

        ws.onmessage = function(e) {{
            var now = Date.now();
            var dt = now - lastUpdate;
            lastUpdate = now;
            document.getElementById('updateRate').textContent = dt + ' ms';
            if (typeof resetWatchdog === 'function') resetWatchdog();
            try {{
                parseBinaryFrame(e.data);
            }} catch(ex) {{
                // fallback: try JSON
                try {{
                    var data = JSON.parse(typeof e.data === 'string' ? e.data : '');
                    for (var key in data) {{
                        if (bindings[key]) bindings[key].forEach(function(fn){{ fn(data[key]); }});
                    }}
                }} catch(ex2) {{}}
            }}
        }};

        ws.onerror = function() {{}};
    }} catch(ex) {{
        scheduleReconnect();
    }}
}}

// --- Binary Frame Parser ---
// VAR_UPDATE (0x01): [2B var_count] then per var: [2B name_len] [name_bytes] [1B type] [4B value] [4B timestamp]
// Wire type codes match firmware plc_value_type_t:
// BOOL=0 BYTE=1 INT=2 DINT=3 REAL=4 TIME=5 STRING=6 WORD=7 DWORD=8
function decodeVarValue(view, offset, varType) {{
    switch (varType) {{
        case 4: return view.getFloat32(offset, true);   // REAL
        case 7: return view.getUint32(offset, true);    // WORD (unsigned)
        case 8: return view.getUint32(offset, true);    // DWORD (unsigned)
        default: return view.getInt32(offset, true);    // BOOL/BYTE/INT/DINT/TIME/STRING
    }}
}}

function parseBinaryFrame(buf) {{
    var view = new DataView(buf);
    var offset = 0;
    if (view.byteLength < 3) return;

    var msgType = view.getUint8(offset);
    offset += 1;

    if (msgType !== 0x01) return;

    var varCount = view.getUint16(offset, true);
    offset += 2;

    for (var i = 0; i < varCount && offset < view.byteLength; i++) {{
        if (offset + 2 > view.byteLength) break;
        var nameLen = view.getUint16(offset, true);
        offset += 2;

        if (offset + nameLen > view.byteLength) break;
        var nameBytes = new Uint8Array(buf, offset, nameLen);
        var name = '';
        for (var j = 0; j < nameLen; j++) name += String.fromCharCode(nameBytes[j]);
        offset += nameLen;

        if (offset + 1 > view.byteLength) break;
        var varType = view.getUint8(offset);
        offset += 1;

        if (offset + 4 > view.byteLength) break;
        var value = decodeVarValue(view, offset, varType);
        offset += 4;

        if (offset + 4 > view.byteLength) break;
        offset += 4;

        if (varType === 0) {{
            buttonStates[name] = value !== 0;
            value = value !== 0;
        }}

        if (bindings[name]) {{
            bindings[name].forEach(function(fn) {{ fn(value); }});
        }}
    }}
}}

// --- Force Variable Write-back ---
// varType must match the target's wire type (BOOL=0 ... REAL=4 ... DWORD=8);
// REAL is written as float32, everything else as int32.
function sendForceVar(name, varType, value) {{
    if (!ws || ws.readyState !== WebSocket.OPEN) return;
    var nameBytes = [];
    for (var i = 0; i < name.length; i++) nameBytes.push(name.charCodeAt(i));
    var buf = new ArrayBuffer(1 + 2 + nameBytes.length + 1 + 4);
    var view = new DataView(buf);
    var offset = 0;
    view.setUint8(offset, 0x02); offset += 1;
    view.setUint16(offset, nameBytes.length, true); offset += 2;
    for (var i = 0; i < nameBytes.length; i++) {{ view.setUint8(offset, nameBytes[i]); offset += 1; }}
    view.setUint8(offset, varType); offset += 1;
    if (varType === 4) {{
        view.setFloat32(offset, Number(value), true);
    }} else {{
        view.setInt32(offset, Number(value), true);
    }}
    offset += 4;
    ws.send(buf);
}}

// --- Button Handler ---
function handleButton(varName, toggleValue, varType) {{
    if (!varName) return;
    buttonStates[varName] = !!toggleValue;
    sendForceVar(varName, varType || 0, toggleValue ? 1 : 0);
}}

// --- Init ---
connectWS();"#,
        plc_ip_js = plc_ip_js,
    )
}

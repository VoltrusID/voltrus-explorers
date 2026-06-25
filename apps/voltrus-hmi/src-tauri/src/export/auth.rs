use crate::*;

/// Generate auth HTML — role indicator in status bar + PIN modal.
pub fn render_auth_html() -> String {
    r#"<div id="pin-modal" style="display:none;position:fixed;inset:0;z-index:200;background:rgba(0,0,0,0.7);display:none;align-items:center;justify-content:center">
    <div style="background:#1e1e2e;border:1px solid #444;border-radius:12px;padding:24px;width:280px;text-align:center">
        <div style="font-size:14px;color:#ccc;margin-bottom:16px">Enter PIN</div>
        <div id="pin-dots" style="display:flex;justify-content:center;gap:8px;margin-bottom:16px">
            <div class="pin-dot" style="width:12px;height:12px;border-radius:50%;background:#444"></div>
            <div class="pin-dot" style="width:12px;height:12px;border-radius:50%;background:#444"></div>
            <div class="pin-dot" style="width:12px;height:12px;border-radius:50%;background:#444"></div>
            <div class="pin-dot" style="width:12px;height:12px;border-radius:50%;background:#444"></div>
        </div>
        <div id="pin-error" style="font-size:11px;color:#ef4444;margin-bottom:8px;min-height:16px"></div>
        <div style="display:grid;grid-template-columns:1fr 1fr 1fr;gap:6px">
            <button onclick="enterPinDigit('1')" style="padding:10px;background:#2a2a3a;color:#ccc;border:1px solid #444;border-radius:6px;font-size:18px;cursor:pointer">1</button>
            <button onclick="enterPinDigit('2')" style="padding:10px;background:#2a2a3a;color:#ccc;border:1px solid #444;border-radius:6px;font-size:18px;cursor:pointer">2</button>
            <button onclick="enterPinDigit('3')" style="padding:10px;background:#2a2a3a;color:#ccc;border:1px solid #444;border-radius:6px;font-size:18px;cursor:pointer">3</button>
            <button onclick="enterPinDigit('4')" style="padding:10px;background:#2a2a3a;color:#ccc;border:1px solid #444;border-radius:6px;font-size:18px;cursor:pointer">4</button>
            <button onclick="enterPinDigit('5')" style="padding:10px;background:#2a2a3a;color:#ccc;border:1px solid #444;border-radius:6px;font-size:18px;cursor:pointer">5</button>
            <button onclick="enterPinDigit('6')" style="padding:10px;background:#2a2a3a;color:#ccc;border:1px solid #444;border-radius:6px;font-size:18px;cursor:pointer">6</button>
            <button onclick="enterPinDigit('7')" style="padding:10px;background:#2a2a3a;color:#ccc;border:1px solid #444;border-radius:6px;font-size:18px;cursor:pointer">7</button>
            <button onclick="enterPinDigit('8')" style="padding:10px;background:#2a2a3a;color:#ccc;border:1px solid #444;border-radius:6px;font-size:18px;cursor:pointer">8</button>
            <button onclick="enterPinDigit('9')" style="padding:10px;background:#2a2a3a;color:#ccc;border:1px solid #444;border-radius:6px;font-size:18px;cursor:pointer">9</button>
            <button onclick="closePinModal()" style="padding:10px;background:#2a2a3a;color:#ef4444;border:1px solid #444;border-radius:6px;font-size:14px;cursor:pointer">X</button>
            <button onclick="enterPinDigit('0')" style="padding:10px;background:#2a2a3a;color:#ccc;border:1px solid #444;border-radius:6px;font-size:18px;cursor:pointer">0</button>
            <button onclick="enterPinBackspace()" style="padding:10px;background:#2a2a3a;color:#f59e0b;border:1px solid #444;border-radius:6px;font-size:14px;cursor:pointer">←</button>
        </div>
    </div>
</div>"#.to_string()
}

/// Generate auth JS — PIN hashing, role enforcement, auto-logout, lockout.
pub fn render_auth_js(config: &AccessConfig) -> String {
    format!(
        r#"
// --- Access Control ---
var ACCESS_CONFIG = {{
    operatorPinHash: "{op_hash}",
    supervisorPinHash: "{sup_hash}",
    salt: "{salt}",
    autoLogoutMinutes: {auto_logout}
}};
var currentRole = 'viewer'; // viewer < operator < supervisor
var pinBuffer = '';
var wrongAttempts = 0;
var lockoutUntil = 0;
var autoLogoutTimer = null;

function hashPin(pin) {{
    var encoder = new TextEncoder();
    var data = encoder.encode(ACCESS_CONFIG.salt + pin);
    return crypto.subtle.digest('SHA-256', data).then(function(hash) {{
        return Array.from(new Uint8Array(hash)).map(function(b) {{ return b.toString(16).padStart(2, '0'); }}).join('');
    }});
}}

function checkRole(required) {{
    var levels = {{ viewer: 0, operator: 1, supervisor: 2 }};
    return (levels[currentRole] || 0) >= (levels[required] || 0);
}}

function applyRoleToControls() {{
    var buttons = document.querySelectorAll('.widget.button button');
    var sliders = document.querySelectorAll('.widget.slider input[type="range"]');
    var roleWidgets = document.querySelectorAll('[data-role]');
    buttons.forEach(function(btn) {{
        btn.style.opacity = currentRole === 'viewer' ? '0.5' : '1';
        btn.style.pointerEvents = currentRole === 'viewer' ? 'none' : 'auto';
    }});
    sliders.forEach(function(slider) {{
        slider.disabled = currentRole === 'viewer';
    }});
    roleWidgets.forEach(function(el) {{
        var req = el.getAttribute('data-role');
        var ok = checkRole(req);
        el.style.opacity = ok ? '1' : '0.5';
        el.style.pointerEvents = ok ? 'auto' : 'none';
    }});
    // Update role indicator
    var indicator = document.getElementById('role-indicator');
    if (indicator) {{
        var icons = {{ viewer: '🔒', operator: '👤', supervisor: '👑' }};
        var labels = {{ viewer: 'Viewer', operator: 'Operator', supervisor: 'Supervisor' }};
        indicator.textContent = (icons[currentRole]||'') + ' ' + (labels[currentRole]||'');
    }}
}}

function showPinModal() {{
    var modal = document.getElementById('pin-modal');
    if (modal) {{ modal.style.display = 'flex'; }}
    pinBuffer = '';
    updatePinDots();
    document.getElementById('pin-error').textContent = '';
}}

function closePinModal() {{
    var modal = document.getElementById('pin-modal');
    if (modal) {{ modal.style.display = 'none'; }}
    pinBuffer = '';
    updatePinDots();
}}

function updatePinDots() {{
    var dots = document.querySelectorAll('.pin-dot');
    dots.forEach(function(dot, i) {{
        dot.style.background = i < pinBuffer.length ? '#22c55e' : '#444';
    }});
}}

function enterPinDigit(d) {{
    if (Date.now() < lockoutUntil) {{
        var remaining = Math.ceil((lockoutUntil - Date.now()) / 1000);
        document.getElementById('pin-error').textContent = 'Locked out. Wait ' + remaining + 's';
        return;
    }}
    if (pinBuffer.length >= 4) return;
    pinBuffer += d;
    updatePinDots();
    if (pinBuffer.length === 4) {{
        hashPin(pinBuffer).then(function(hash) {{
            if (hash === ACCESS_CONFIG.supervisorPinHash) {{
                currentRole = 'supervisor';
            }} else if (hash === ACCESS_CONFIG.operatorPinHash) {{
                currentRole = 'operator';
            }} else {{
                wrongAttempts++;
                if (wrongAttempts >= 3) {{
                    lockoutUntil = Date.now() + 30000;
                    document.getElementById('pin-error').textContent = '3 wrong attempts. Wait 30s.';
                }} else {{
                    document.getElementById('pin-error').textContent = 'Wrong PIN (' + wrongAttempts + '/3)';
                }}
                pinBuffer = '';
                updatePinDots();
                return;
            }}
            wrongAttempts = 0;
            closePinModal();
            applyRoleToControls();
            startAutoLogoutTimer();
        }});
    }}
}}

function enterPinBackspace() {{
    pinBuffer = pinBuffer.slice(0, -1);
    updatePinDots();
}}

function startAutoLogoutTimer() {{
    if (autoLogoutTimer) clearTimeout(autoLogoutTimer);
    if (currentRole === 'viewer') return;
    autoLogoutTimer = setTimeout(function() {{
        currentRole = 'viewer';
        applyRoleToControls();
    }}, ACCESS_CONFIG.autoLogoutMinutes * 60 * 1000);
}}

// Reset timer on any interaction
document.addEventListener('click', function() {{ startAutoLogoutTimer(); }});
document.addEventListener('touchstart', function() {{ startAutoLogoutTimer(); }});

// Initialize as viewer
applyRoleToControls();
"#,
        op_hash = config.operator_pin_hash,
        sup_hash = config.supervisor_pin_hash,
        salt = config.salt,
        auto_logout = config.auto_logout_minutes,
    )
}

/// Generate role indicator HTML for status bar.
pub fn render_role_indicator_html() -> String {
    r#"<span id="role-indicator" style="cursor:pointer" onclick="showPinModal()">🔒 Viewer</span>"#.to_string()
}

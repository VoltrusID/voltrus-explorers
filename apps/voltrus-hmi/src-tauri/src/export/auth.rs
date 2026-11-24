use super::js_escape;
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
///
/// Hashing uses a pure-JS SHA-256 (no `crypto.subtle`) so PIN login also works
/// when the export is served over plain HTTP, where subtle crypto is undefined.
/// The PIN is stretched (SHA-256 chained 5000 rounds) so the 4-digit keyspace
/// cannot be brute-forced from the hashes embedded in the file. PinSetup.tsx
/// in the designer computes the identical `stretchPin` — keep them in sync.
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

// Lockout survives page reloads within the session.
try {{
    wrongAttempts = parseInt(sessionStorage.getItem('pinAttempts') || '0', 10) || 0;
    lockoutUntil = parseInt(sessionStorage.getItem('pinLockoutUntil') || '0', 10) || 0;
}} catch (e) {{}}

function persistPinState() {{
    try {{
        sessionStorage.setItem('pinAttempts', String(wrongAttempts));
        sessionStorage.setItem('pinLockoutUntil', String(lockoutUntil));
    }} catch (e) {{}}
}}

// --- Pure-JS SHA-256 (ASCII input only; salt and PIN digits are ASCII) ---
function sha256Hex(ascii) {{
    function rightRotate(value, amount) {{ return (value >>> amount) | (value << (32 - amount)); }}
    var maxWord = Math.pow(2, 32);
    var result = '';
    var words = [];
    var asciiBitLength = ascii.length * 8;
    var hash = sha256Hex.h = sha256Hex.h || [];
    var k = sha256Hex.k = sha256Hex.k || [];
    var primeCounter = k.length;
    var isComposite = {{}};
    for (var candidate = 2; primeCounter < 64; candidate++) {{
        if (!isComposite[candidate]) {{
            for (var i = 0; i < 313; i += candidate) {{ isComposite[i] = candidate; }}
            hash[primeCounter] = (Math.pow(candidate, 0.5) * maxWord) | 0;
            k[primeCounter++] = (Math.pow(candidate, 1 / 3) * maxWord) | 0;
        }}
    }}
    ascii += '\x80';
    while (ascii.length % 64 - 56) ascii += '\x00';
    for (var i = 0; i < ascii.length; i++) {{
        var j = ascii.charCodeAt(i);
        if (j >> 8) return '';
        words[i >> 2] |= j << ((3 - i) % 4) * 8;
    }}
    words[words.length] = (asciiBitLength / maxWord) | 0;
    words[words.length] = asciiBitLength;
    for (var j = 0; j < words.length;) {{
        var w = words.slice(j, j += 16);
        var oldHash = hash;
        hash = hash.slice(0, 8);
        for (var i = 0; i < 64; i++) {{
            var w15 = w[i - 15], w2 = w[i - 2];
            var a = hash[0], e = hash[4];
            var temp1 = hash[7]
                + (rightRotate(e, 6) ^ rightRotate(e, 11) ^ rightRotate(e, 25))
                + ((e & hash[5]) ^ ((~e) & hash[6]))
                + k[i]
                + (w[i] = (i < 16) ? w[i] : (
                        w[i - 16]
                        + (rightRotate(w15, 7) ^ rightRotate(w15, 18) ^ (w15 >>> 3))
                        + w[i - 7]
                        + (rightRotate(w2, 17) ^ rightRotate(w2, 19) ^ (w2 >>> 10))
                    ) | 0
                );
            var temp2 = (rightRotate(a, 2) ^ rightRotate(a, 13) ^ rightRotate(a, 22))
                + ((a & hash[1]) ^ (a & hash[2]) ^ (hash[1] & hash[2]));
            hash = [(temp1 + temp2) | 0].concat(hash);
            hash[4] = (hash[4] + temp1) | 0;
        }}
        for (var i = 0; i < 8; i++) {{
            hash[i] = (hash[i] + oldHash[i]) | 0;
        }}
    }}
    for (var i = 0; i < 8; i++) {{
        for (var j = 3; j + 1; j--) {{
            var b = (hash[i] >> (j * 8)) & 255;
            result += ((b < 16) ? 0 : '') + b.toString(16);
        }}
    }}
    return result;
}}

// Stretch: chain SHA-256 5000 times so the 10^4 PIN keyspace is
// computationally expensive to search offline.
function stretchPin(salt, pin) {{
    var h = sha256Hex(salt + pin);
    for (var i = 0; i < 5000; i++) h = sha256Hex(h);
    return h;
}}

function hashPin(pin) {{
    return Promise.resolve(stretchPin(ACCESS_CONFIG.salt, pin));
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
        var hash;
        try {{
            hash = hashPin(pinBuffer);
        }} catch (ex) {{
            document.getElementById('pin-error').textContent = 'Hash error';
            pinBuffer = '';
            updatePinDots();
            return;
        }}
        hash.then(function(h) {{
            if (h === ACCESS_CONFIG.supervisorPinHash) {{
                currentRole = 'supervisor';
            }} else if (h === ACCESS_CONFIG.operatorPinHash) {{
                currentRole = 'operator';
            }} else {{
                wrongAttempts++;
                if (wrongAttempts >= 3) {{
                    lockoutUntil = Date.now() + 30000;
                    document.getElementById('pin-error').textContent = '3 wrong attempts. Wait 30s.';
                }} else {{
                    document.getElementById('pin-error').textContent = 'Wrong PIN (' + wrongAttempts + '/3)';
                }}
                persistPinState();
                pinBuffer = '';
                updatePinDots();
                return;
            }}
            wrongAttempts = 0;
            persistPinState();
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
        op_hash = js_escape(&config.operator_pin_hash),
        sup_hash = js_escape(&config.supervisor_pin_hash),
        salt = js_escape(&config.salt),
        auto_logout = config.auto_logout_minutes,
    )
}

/// Generate role indicator HTML for status bar.
pub fn render_role_indicator_html() -> String {
    r#"<span id="role-indicator" style="cursor:pointer" onclick="showPinModal()">🔒 Viewer</span>"#
        .to_string()
}

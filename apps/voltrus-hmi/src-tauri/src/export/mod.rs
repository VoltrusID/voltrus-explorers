pub mod alarm;
pub mod auth;
pub mod i18n;
pub mod kiosk;
pub mod widgets;
pub mod ws_runtime;

use crate::*;

/// Escape strings for safe embedding inside JS string literals.
fn js_escape(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\'', "\\'")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
        .replace('`', "\\`")
        .replace('<', "\\x3c")
        .replace('>', "\\x3e")
}

/// Escape strings for safe embedding in HTML text content and attributes.
fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

/// Make a serde_json string safe to embed inside a `<script>` block.
/// serde_json does not escape `<`/`>`, so a value containing `</script>` would
/// break out of the element; JS unicode escapes are valid inside JSON strings.
fn json_for_script(json: &str) -> String {
    json.replace('<', "\\u003c")
        .replace('>', "\\u003e")
        .replace('\u{2028}', "\\u2028")
        .replace('\u{2029}', "\\u2029")
}

/// Validate a CSS color: only `#rgb`-style hex is accepted, anything else
/// falls back to `fallback` so a crafted project file cannot inject style rules.
fn safe_color<'a>(s: &'a str, fallback: &'a str) -> &'a str {
    let valid = s.starts_with('#')
        && s.len() >= 4
        && s.len() <= 9
        && s[1..].chars().all(|c| c.is_ascii_hexdigit());
    if valid {
        s
    } else {
        fallback
    }
}

/// Validate a PLC host (`host` or `host:port`) before it is interpolated into
/// URLs, JS strings, and HTML. Anything unexpected falls back to the default.
fn safe_host(s: &str) -> &str {
    let valid = !s.is_empty()
        && s.len() <= 255
        && !s.contains("..")
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | ':' | '-' | '_'));
    if valid {
        s
    } else {
        "192.168.1.100"
    }
}

/// Widget ids end up in HTML id attributes and JS getElementById strings;
/// restrict to a conservative charset so hand-edited project files cannot
/// inject markup or break the generated script.
pub(crate) fn safe_id(s: &str) -> String {
    let cleaned: String = s
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    if cleaned.is_empty() {
        "w".to_string()
    } else {
        cleaned
    }
}

/// Map a declared PLC data type to the wire type code used by the runtime
/// (see firmware `plc_value_type_t`: BOOL=0 BYTE=1 INT=2 DINT=3 REAL=4
/// TIME=5 STRING=6 WORD=7 DWORD=8). Unknown types write as DINT.
pub(crate) fn var_type_code(data_type: Option<&str>) -> u8 {
    match data_type.unwrap_or("").to_ascii_uppercase().as_str() {
        "BOOL" => 0,
        "BYTE" => 1,
        "INT" => 2,
        "DINT" => 3,
        "REAL" | "FLOAT" => 4,
        "TIME" => 5,
        "STRING" => 6,
        "WORD" => 7,
        "DWORD" => 8,
        _ => 3,
    }
}

/// Only known roles may reach the exported HTML.
fn safe_role(r: Option<&str>) -> &'static str {
    match r {
        Some("operator") => "operator",
        Some("supervisor") => "supervisor",
        _ => "viewer",
    }
}

pub fn build_export_html(request: &ExportRequest) -> Result<String, String> {
    let plc_ip = safe_host(if request.plc_ip.is_empty() {
        "192.168.1.100"
    } else {
        &request.plc_ip
    })
    .to_string();

    let has_alarms = !request.alarms.is_empty() || request.scada_url.is_some();
    let has_auth = request.access_config.is_some();
    let has_i18n = request
        .translations
        .as_ref()
        .map(|t| t.languages.len() > 1)
        .unwrap_or(false);

    let mut screen_sections: Vec<String> = Vec::new();
    let mut nav_buttons: Vec<String> = Vec::new();
    let mut all_bindings_js = String::new();

    for (screen_idx, screen) in request.screens.iter().enumerate() {
        let screen_class = if screen_idx == 0 { "screen active" } else { "screen" };
        let name_html = html_escape(&screen.name);
        nav_buttons.push(format!(
            r#"<button class="nav-btn{active}" onclick="switchScreen({idx})" data-screen="{idx}" data-i18n="{name_html}">{name_html}</button>"#,
            idx = screen_idx,
            name_html = name_html,
            active = if screen_idx == 0 { " active" } else { "" },
        ));

        let mut widgets_html = String::new();

        for widget in &screen.widgets {
            let mut var_access = if widget.variable_name.is_empty() {
                String::new()
            } else {
                format!(r#"data-bind="{}""#, html_escape(&widget.variable_name))
            };

            if has_auth {
                var_access.push_str(&format!(r#" data-role="{}""#, safe_role(widget.required_role.as_deref())));
            }

            match widget.widget_type.as_str() {
                "gauge" => {
                    widgets_html.push_str(&widgets::render_gauge_html(widget, &var_access));
                    if !widget.variable_name.is_empty() {
                        all_bindings_js.push_str(&widgets::render_gauge_js(widget));
                    }
                }
                "chart" => {
                    widgets_html.push_str(&widgets::render_chart_html(widget, &var_access));
                    if !widget.variable_name.is_empty() {
                        all_bindings_js.push_str(&widgets::render_chart_js(widget));
                    }
                }
                "indicator" => {
                    widgets_html.push_str(&widgets::render_indicator_html(widget, &var_access));
                    if !widget.variable_name.is_empty() {
                        all_bindings_js.push_str(&widgets::render_indicator_js(widget));
                    }
                }
                "button" => {
                    widgets_html.push_str(&widgets::render_button_html(widget, &var_access));
                }
                "slider" => {
                    widgets_html.push_str(&widgets::render_slider_html(widget, &var_access));
                    if !widget.variable_name.is_empty() {
                        all_bindings_js.push_str(&widgets::render_slider_js(widget));
                    }
                }
                "text" => {
                    widgets_html.push_str(&widgets::render_text_html(widget, &var_access));
                    if !widget.variable_name.is_empty() {
                        all_bindings_js.push_str(&widgets::render_text_js(widget));
                    }
                }
                _ => {}
            }
        }

        screen_sections.push(format!(
            r#"<div class="{screen_class}" id="screen-{idx}" style="position:relative;width:100%;height:calc(100vh - 68px);overflow:auto;background:{bg}">
{widgets}
</div>"#,
            screen_class = screen_class,
            idx = screen_idx,
            bg = safe_color(&screen.background_color, "#1e1e2e"),
            widgets = widgets_html,
        ));
    }

    let lang_dropdown = if has_i18n {
        i18n::render_language_dropdown_html(
            request.translations.as_ref().unwrap(),
            request.default_language.as_deref().unwrap_or("en"),
        )
    } else {
        String::new()
    };

    let nav_bar = if request.screens.len() > 1 || !lang_dropdown.is_empty() {
        format!(
            r#"<div class="nav-bar">{}</div>"#,
            if request.screens.len() > 1 {
                format!("{}{}", nav_buttons.join("\n"), lang_dropdown)
            } else {
                lang_dropdown
            }
        )
    } else {
        String::new()
    };
    // Without a nav bar the screens must start at the top edge, not below one.
    let screen_top = if nav_bar.is_empty() { 0 } else { 40 };

    // Build feature-flagged JS blocks
    let alarm_html = if has_alarms {
        alarm::render_alarm_banner_html()
    } else {
        String::new()
    };
    let alarm_js = if has_alarms {
        alarm::render_alarm_banner_js(
            &request.alarms,
            request.scada_url.as_deref().unwrap_or(""),
        )
    } else {
        String::new()
    };

    let auth_html = if has_auth {
        auth::render_auth_html()
    } else {
        String::new()
    };
    let auth_js = if let Some(ref config) = request.access_config {
        auth::render_auth_js(config)
    } else {
        String::new()
    };
    let role_indicator = if has_auth {
        auth::render_role_indicator_html()
    } else {
        String::new()
    };

    let i18n_js = if has_i18n {
        i18n::render_i18n_js(
            request.translations.as_ref().unwrap(),
            request.default_language.as_deref().unwrap_or("en"),
        )
    } else {
        String::new()
    };

    let kiosk_html = if request.kiosk_mode {
        kiosk::render_kiosk_overlay_html()
    } else {
        String::new()
    };
    let kiosk_js = if request.kiosk_mode {
        kiosk::render_kiosk_js()
    } else {
        String::new()
    };
    let fullscreen_btn = if request.kiosk_mode {
        kiosk::render_fullscreen_button_html()
    } else {
        String::new()
    };

    let screens_html = screen_sections.join("\n");
    let ws_js = ws_runtime::render_ws_runtime_js(&plc_ip);

    let html = format!(
        r##"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="UTF-8">
<meta name="viewport" content="width=device-width,initial-scale=1.0">
<title>Voltrus HMI</title>
<style>
*{{margin:0;padding:0;box-sizing:border-box}}
body{{background:#1e1e2e;font-family:-apple-system,BlinkMacSystemFont,sans-serif;color:#ccc;overflow:hidden;width:100vw;height:100vh;position:relative}}
.widget{{user-select:none}}
.nav-bar{{position:fixed;top:0;left:0;right:0;height:40px;background:#111;display:flex;align-items:center;padding:0 8px;gap:4px;z-index:100;border-bottom:1px solid #333}}
.nav-btn{{background:#2a2a3a;color:#ccc;border:1px solid #444;border-radius:4px;padding:6px 14px;font-size:12px;cursor:pointer;transition:all 0.2s}}
.nav-btn:hover{{background:#3a3a4a}}
.nav-btn.active{{background:{nav_active_bg};color:#fff;border-color:#666}}
.status-bar{{position:fixed;bottom:0;left:0;right:0;height:28px;background:#111;display:flex;align-items:center;padding:0 12px;font-size:11px;color:#666;gap:16px;z-index:100;border-top:1px solid #333}}
.status-dot{{width:8px;height:8px;border-radius:50%;display:inline-block;margin-right:4px}}
.status-dot.connected{{background:#22c55e;box-shadow:0 0 6px #22c55e}}
.status-dot.disconnected{{background:#666}}
.screen{{opacity:0;transition:opacity 0.3s ease;pointer-events:none;position:absolute;top:{screen_top}px;left:0;right:0;bottom:28px}}
.screen.active{{opacity:1;pointer-events:auto}}
</style>
</head>
<body>
{nav_bar}
{alarm_html}
{screens_html}
{auth_html}
{kiosk_html}
<div class="status-bar">
<span><span class="status-dot disconnected" id="wsStatus"></span><span id="wsLabel">Connecting...</span> PLC: {plc_ip_html}</span>
<span id="varCount">0 variables bound</span>
<span id="updateRate">-- ms</span>
{role_indicator}
{fullscreen_btn}
</div>
<script>
// --- Bindings & State ---
var bindings = {{}};
var chartData = {{}};
var buttonStates = {{}};
var currentScreen = 0;

{bindings_js}

var varCount = Object.keys(bindings).length;
document.getElementById('varCount').textContent = varCount + ' variables bound';

{ws_js}

// --- Feature-flagged blocks ---
{alarm_js}
{auth_js}
{i18n_js}
{kiosk_js}
</script>
</body>
</html>"##,
        nav_bar = nav_bar,
        alarm_html = alarm_html,
        screens_html = screens_html,
        auth_html = auth_html,
        kiosk_html = kiosk_html,
        plc_ip_html = html_escape(&plc_ip),
        bindings_js = all_bindings_js,
        role_indicator = role_indicator,
        fullscreen_btn = fullscreen_btn,
        nav_active_bg = safe_color(
            request
                .screens
                .first()
                .map(|s| s.background_color.as_str())
                .unwrap_or("#2a2a3a"),
            "#2a2a3a"
        ),
        screen_top = screen_top,
    );

    Ok(html)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn widget(id: &str, var: &str, wtype: &str) -> HmiWidget {
        HmiWidget {
            id: id.into(),
            widget_type: wtype.into(),
            x: 0.0,
            y: 0.0,
            width: 100.0,
            height: 100.0,
            label: "Lab</script>".into(),
            variable_name: var.into(),
            data_type: Some("REAL".into()),
            min: 0.0,
            max: 100.0,
            color: "#22c55e".into(),
            font_size: 14.0,
            required_role: None,
            z_index: None,
        }
    }

    fn request(plc_ip: &str, bg: &str) -> ExportRequest {
        ExportRequest {
            screens: vec![HmiScreen {
                id: "s1".into(),
                name: "Scr</script>&amp;".into(),
                background_color: bg.into(),
                widgets: vec![
                    widget("w1'</i>", "var1", "slider"),
                    widget("w2", "var2", "indicator"),
                ],
            }],
            plc_ip: plc_ip.into(),
            scada_url: None,
            alarms: vec![AlarmRule {
                id: "a1".into(),
                source: "plc".into(),
                variable_name: "var2".into(),
                severity: "critical".into(),
                message: "</script><script>alert(1)</script>".into(),
            }],
            access_config: None,
            translations: None,
            default_language: None,
            kiosk_mode: false,
        }
    }

    #[test]
    fn escapes_script_breakout_in_labels_and_alarm_json() {
        let html = build_export_html(&request("10.0.0.5", "#1e1e2e")).unwrap();
        // The raw breakout payload must never appear in the document.
        assert!(!html.contains("</script><script>"), "alarm JSON leaked script breakout");
        // The label payload must not appear unescaped either.
        assert!(!html.contains("Lab</script>"), "widget label leaked script breakout");
        assert!(html.contains("Lab&lt;/script&gt;"));
    }

    #[test]
    fn rejects_malicious_host_and_color() {
        let html = build_export_html(&request("evil'\"}));alert(1);//", "red;}body{display:none")).unwrap();
        assert!(html.contains("ws://192.168.1.100/ws/plc"), "host fell through validation");
        assert!(!html.contains("ws://evil"), "host injection survived");
        assert!(html.contains("#1e1e2e"), "invalid color did not fall back to default");
        assert!(!html.contains("display:none}"), "css injection survived");
    }

    #[test]
    fn sanitizes_widget_ids() {
        let html = build_export_html(&request("10.0.0.5", "#1e1e2e")).unwrap();
        assert!(html.contains("ind-w2") || html.contains("chart-line-w"), "expected sanitized ids");
        assert!(!html.contains("w1'</i>'"), "raw id with quote survived");
    }


    #[test]
    fn typed_writes_use_var_type_code() {
        let html = build_export_html(&request("10.0.0.5", "#1e1e2e")).unwrap();
        // REAL slider must send wire type 4, not the old hardcoded 0.
        assert!(html.contains("sendForceVar('var1',4,this.value)"), "slider write not typed");
    }

    #[test]
    fn full_featured_export_builds() {
        let mut req = request("10.0.0.5", "#1e1e2e");
        req.screens.push(HmiScreen {
            id: "s2".into(),
            name: "Pump & Tank".into(),
            background_color: "#101018".into(),
            widgets: vec![widget("w3", "level_setpoint", "button")],
        });
        req.scada_url = Some("http://scada.local:8080".into());
        req.access_config = Some(AccessConfig {
            operator_pin_hash: "aa".repeat(32),
            supervisor_pin_hash: "bb".repeat(32),
            salt: "deadbeef".into(),
            auto_logout_minutes: 5,
        });
        req.translations = Some(TranslationSet {
            languages: [
                ("en".to_string(), [("Pump & Tank".to_string(), "Pump & Tank".to_string())].into_iter().collect()),
                ("id".to_string(), [("Pump & Tank".to_string(), "Pompa & Tangki".to_string())].into_iter().collect()),
            ]
            .into_iter().collect(),
        });
        req.default_language = Some("id".into());
        req.kiosk_mode = true;
        let html = build_export_html(&req).unwrap();
        // Handy for manual inspection: /tmp/voltrus-hmi-export-test.html
        let _ = std::fs::write("/tmp/voltrus-hmi-export-test.html", &html);
        // All feature blocks present
        assert!(html.contains("pin-modal"), "auth block missing");
        assert!(html.contains("lang-select"), "i18n dropdown missing");
        assert!(html.contains("connection-lost"), "kiosk overlay missing");
        assert!(html.contains("connectSCADAAlarms"), "scada polling missing");
        assert!(html.contains("stretchPin"), "stretched pin hashing missing");
        // Translations must be script-safe (no raw </script> from the key)
        assert!(!html.contains("Pompa & Tangki</"), "unsafe translation output");
    }
}

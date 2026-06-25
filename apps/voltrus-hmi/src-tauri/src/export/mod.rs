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

pub fn build_export_html(request: &ExportRequest) -> Result<String, String> {
    let plc_ip = if request.plc_ip.is_empty() {
        "192.168.1.100".to_string()
    } else {
        request.plc_ip.clone()
    };

    let has_alarms = !request.alarms.is_empty() || request.scada_url.is_some();
    let has_auth = request.access_config.is_some();
    let has_i18n = request.translations.is_some()
        && request
            .translations
            .as_ref()
            .map(|t| t.languages.len() > 1)
            .unwrap_or(false);

    let mut screen_sections: Vec<String> = Vec::new();
    let mut nav_buttons: Vec<String> = Vec::new();
    let mut all_bindings_js = String::new();

    for (screen_idx, screen) in request.screens.iter().enumerate() {
        let screen_class = if screen_idx == 0 { "screen active" } else { "screen" };
        nav_buttons.push(format!(
            r#"<button class="nav-btn{active}" onclick="switchScreen({idx})" data-screen="{idx}" data-i18n="{name_html}">{name_html}</button>"#,
            idx = screen_idx,
            name_html = html_escape(&screen.name),
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
                var_access.push_str(&format!(
                    r#" data-role="{}""#,
                    widget
                        .required_role
                        .as_deref()
                        .filter(|r| *r != "none")
                        .unwrap_or("viewer")
                ));
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
                    widgets_html.push_str(&widgets::render_button_html(widget));
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
            bg = screen.background_color,
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
.screen{{opacity:0;transition:opacity 0.3s ease;pointer-events:none;position:absolute;top:40px;left:0;right:0;bottom:28px}}
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
<span><span class="status-dot disconnected" id="wsStatus"></span><span id="wsLabel">Connecting...</span> PLC: {plc_ip}</span>
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
        plc_ip = plc_ip,
        bindings_js = all_bindings_js,
        role_indicator = role_indicator,
        fullscreen_btn = fullscreen_btn,
        nav_active_bg = request
            .screens
            .first()
            .map(|s| s.background_color.as_str())
            .unwrap_or("#1e1e2e"),
    );

    Ok(html)
}

use crate::*;
use super::{html_escape, js_escape, safe_color, safe_id, var_type_code};

/// A JS string literal that will be placed inside a double-quoted HTML
/// attribute (onclick/oninput): js-escape for the literal, html-escape for the
/// attribute context.
fn attr_js(s: &str) -> String {
    html_escape(&js_escape(s))
}

/// Format an f64 for interpolation into generated JS numbers. `inf`/`NaN`
/// are not valid JS in this context — clamp to the fallback.
fn js_num(v: f64, fallback: f64) -> String {
    if v.is_finite() {
        format!("{}", v)
    } else {
        format!("{}", fallback)
    }
}

/// Number format mode for live values, shared with the designer preview
/// (`formatValue` in WidgetPreviews.tsx): REAL/LREAL/FLOAT show one decimal,
/// the listed integer types show whole numbers, untyped or unknown types show
/// integers as-is and fractions with one decimal. Consumed by `fmtNum` in the runtime.
pub(crate) fn num_mode(data_type: Option<&str>) -> &'static str {
    match data_type.unwrap_or("").to_ascii_uppercase().as_str() {
        "REAL" | "LREAL" | "FLOAT" => "real",
        "BOOL" | "BYTE" | "SINT" | "USINT" | "INT" | "UINT" | "DINT" | "UDINT" | "LINT"
        | "ULINT" | "WORD" | "DWORD" | "LWORD" | "TIME" => "int",
        // Untyped or unknown (e.g. FLOAT64): never round away a fraction.
        _ => "auto",
    }
}

/// Generate HTML for a gauge widget using inline SVG with semicircular arc.
pub fn render_gauge_html(widget: &HmiWidget, var_access: &str) -> String {
    let cx = 100.0;
    let cy = 110.0;
    let radius = 80.0;
    let id = safe_id(&widget.id);
    let color = safe_color(&widget.color, "#22c55e");

    let bg_arc = format!(
        "M {x1} {y1} A {r} {r} 0 0 1 {x2} {y2}",
        x1 = cx - radius,
        y1 = cy,
        r = radius,
        x2 = cx + radius,
        y2 = cy
    );

    let dash_total = std::f64::consts::PI * radius;
    let text_y = cy - 25.0;
    let z_index_style = widget.z_index.map_or(String::new(), |z| format!("z-index:{};", z));

    let mut s = String::new();
    s.push_str(&format!(
        "<div class=\"widget gauge\" style=\"left:{}px;top:{}px;width:{}px;height:{}px;",
        widget.x, widget.y, widget.width, widget.height
    ));
    s.push_str(&z_index_style);
    s.push_str("background:#2a2a3a;border-radius:8px;position:absolute;");
    s.push_str("display:flex;flex-direction:column;align-items:center;justify-content:center\" ");
    s.push_str(var_access);
    s.push_str(">\n");

    s.push_str("    <svg viewBox=\"0 0 200 120\" style=\"width:100%;height:70%\">\n");
    s.push_str("        <path d=\"");
    s.push_str(&bg_arc);
    s.push_str("\" fill=\"none\" stroke=\"#444\" stroke-width=\"12\" stroke-linecap=\"round\"/>\n");

    s.push_str("        <path id=\"gauge-fill-");
    s.push_str(&id);
    s.push_str("\" d=\"");
    s.push_str(&bg_arc);
    s.push_str("\" fill=\"none\" stroke=\"");
    s.push_str(color);
    s.push_str("\" stroke-width=\"12\" stroke-linecap=\"round\"\n");
    s.push_str("              stroke-dasharray=\"");
    s.push_str(&format!("{}", dash_total));
    s.push_str("\" stroke-dashoffset=\"");
    s.push_str(&format!("{}", dash_total));
    s.push_str("\"/>\n");

    s.push_str("        <text x=\"");
    s.push_str(&format!("{}", cx));
    s.push_str("\" y=\"");
    s.push_str(&format!("{}", text_y));
    s.push_str("\" text-anchor=\"middle\" dominant-baseline=\"middle\"\n");
    s.push_str("              id=\"gauge-val-");
    s.push_str(&id);
    s.push_str("\" fill=\"");
    s.push_str(color);
    s.push_str("\" font-size=\"28\" font-weight=\"bold\">--</text>\n");

    s.push_str("    </svg>\n");
    s.push_str("    <span style=\"font-size:11px;color:#999\" data-i18n=\"");
    s.push_str(&html_escape(&widget.label));
    s.push_str("\">");
    s.push_str(&html_escape(&widget.label));
    s.push_str("</span>\n");

    s.push_str("    <span style=\"font-size:10px;color:#666\">");
    s.push_str(&format!("{} - {}", js_num(widget.min, 0.0), js_num(widget.max, 100.0)));
    s.push_str("</span>\n");

    s.push_str("</div>");
    s
}

/// Generate JS binding for gauge value updates.
pub fn render_gauge_js(widget: &HmiWidget) -> String {
    let var = js_escape(&widget.variable_name);
    let id = safe_id(&widget.id);
    let dash_total = std::f64::consts::PI * 80.0;
    let min = js_num(widget.min, 0.0);
    let max = js_num(widget.max, 100.0);

    format!(
        r#"bindings["{var}"] = bindings["{var}"] || [];
bindings["{var}"].push(function(v) {{
    var el = document.getElementById('gauge-val-{id}');
    var fill = document.getElementById('gauge-fill-{id}');
    if (el) el.textContent = fmtNum(v, '{mode}');
    if (fill && typeof v === 'number') {{
        var range = ({max} - {min}) || 1;
        var pct = Math.max(0, Math.min(1, (v - {min}) / range));
        var dashTotal = {dash_total};
        fill.style.strokeDashoffset = dashTotal * (1 - pct);
        if (pct < 0.8) fill.style.stroke = '#22c55e';
        else if (pct < 0.95) fill.style.stroke = '#eab308';
        else fill.style.stroke = '#ef4444';
    }}
}});"#,
        var = var,
        id = id,
        min = min,
        max = max,
        dash_total = dash_total,
        mode = num_mode(widget.data_type.as_deref()),
    )
}

/// Generate HTML for a chart widget using inline SVG polyline.
pub fn render_chart_html(widget: &HmiWidget, var_access: &str) -> String {
    let id = safe_id(&widget.id);
    let color = safe_color(&widget.color, "#22c55e");
    let z_index_style = widget.z_index.map_or(String::new(), |z| format!("z-index:{};", z));
    format!(
        r#"<div class="widget chart" style="left:{x}px;top:{y}px;width:{w}px;height:{h}px;position:absolute;{z}background:#2a2a3a;border:1px solid #444;border-radius:4px;display:flex;flex-direction:column;align-items:center;padding:4px" {var}>
    <span style="font-size:11px;color:#999;flex-shrink:0" data-i18n="{label_attr}">{label}</span>
    <svg id="chart-svg-{id}" viewBox="0 0 200 80" preserveAspectRatio="none" style="width:100%;flex:1;min-height:0">
        <polyline id="chart-line-{id}" fill="none" stroke="{color}" stroke-width="2" vector-effect="non-scaling-stroke" stroke-linejoin="round" points=""/>
    </svg>
</div>"#,
        x = widget.x,
        y = widget.y,
        w = widget.width,
        h = widget.height,
        color = color,
        var = var_access,
        id = id,
        label = html_escape(&widget.label),
        label_attr = html_escape(&widget.label),
        z = z_index_style,
    )
}

/// Generate JS binding for chart value updates (keeps last 50 values).
/// The buffer is per chart widget: two charts on the same variable must not
/// both append to one shared buffer (every sample would land twice).
pub fn render_chart_js(widget: &HmiWidget) -> String {
    let var = js_escape(&widget.variable_name);
    let id = safe_id(&widget.id);

    format!(
        r#"chartData["{id}"] = [];
bindings["{var}"] = bindings["{var}"] || [];
bindings["{var}"].push(function(v) {{
    if (typeof v !== 'number') return;
    var data = chartData["{id}"];
    data.push(v);
    if (data.length > 50) data.shift();
    var line = document.getElementById('chart-line-{id}');
    if (!line || data.length < 2) return;
    var minV = Math.min.apply(null, data);
    var maxV = Math.max.apply(null, data);
    var range = maxV - minV || 1;
    var pts = [];
    for (var i = 0; i < data.length; i++) {{
        var px = (i / (data.length - 1)) * 200;
        var py = 80 - ((data[i] - minV) / range) * 76 - 2;
        pts.push(px.toFixed(1) + ',' + py.toFixed(1));
    }}
    line.setAttribute('points', pts.join(' '));
}});"#,
        var = var,
        id = id,
    )
}

/// Generate HTML for an indicator widget (circle with glow when ON).
pub fn render_indicator_html(widget: &HmiWidget, var_access: &str) -> String {
    let id = safe_id(&widget.id);
    let color = safe_color(&widget.color, "#22c55e");
    let z_index_style = widget.z_index.map_or(String::new(), |z| format!("z-index:{};", z));
    format!(
        r##"<div class="widget indicator" style="left:{x}px;top:{y}px;width:{w}px;height:{h}px;position:absolute;{z}display:flex;flex-direction:column;align-items:center;justify-content:center" {var}>
    <svg class="indicator-light" viewBox="0 0 60 60" style="width:60%;max-height:60%;overflow:visible"><circle id="ind-{id}" cx="30" cy="30" r="27" fill="#333" stroke="{color}" stroke-width="2" style="transition:fill 0.3s,filter 0.3s"/></svg>
    <span style="font-size:{fs}px;color:#ccc;margin-top:4px" data-i18n="{label_attr}">{label}</span>
</div>"##,
        x = widget.x,
        y = widget.y,
        w = widget.width,
        h = widget.height,
        color = color,
        var = var_access,
        id = id,
        fs = widget.font_size,
        label = html_escape(&widget.label),
        label_attr = html_escape(&widget.label),
        z = z_index_style,
    )
}

/// Generate JS binding for indicator value updates.
pub fn render_indicator_js(widget: &HmiWidget) -> String {
    let var = js_escape(&widget.variable_name);
    let id = safe_id(&widget.id);
    let color = js_escape(safe_color(&widget.color, "#22c55e"));

    format!(
        r#"bindings["{var}"] = bindings["{var}"] || [];
bindings["{var}"].push(function(v) {{
    var el = document.getElementById('ind-{id}');
    if (el) {{
        var on = v === true || v === 1 || v === '1' || v === 'true';
        el.setAttribute('fill', on ? '{color}' : '#333');
        el.style.filter = on ? 'drop-shadow(0 0 6px {color})' : 'none';
    }}
}});"#,
        var = var,
        id = id,
        color = color,
    )
}

/// Generate HTML for a button widget with toggle support.
pub fn render_button_html(widget: &HmiWidget, var_access: &str) -> String {
    let id = safe_id(&widget.id);
    let color = safe_color(&widget.color, "#22c55e");
    let var_attr = attr_js(&widget.variable_name);
    let type_code = var_type_code(widget.data_type.as_deref());
    let toggle_expr = if widget.variable_name.is_empty() {
        "1".to_string()
    } else {
        format!("(buttonStates['{var}'] ? 0 : 1)", var = var_attr)
    };
    let z_index_style = widget.z_index.map_or(String::new(), |z| format!("z-index:{};", z));

    format!(
        r#"<div class="widget button" style="left:{x}px;top:{y}px;width:{w}px;height:{h}px;position:absolute;{z}" {var_access}>
    <button id="btn-{id}" onclick="handleButton('{var}',{toggle},{type_code})" style="width:100%;height:100%;background:#2a2a3a;color:{color};border:2px solid {color};border-radius:6px;font-size:{fs}px;font-weight:600;cursor:pointer;transition:all 0.2s" data-i18n="{label_attr}">{label}</button>
</div>"#,
        x = widget.x,
        y = widget.y,
        w = widget.width,
        h = widget.height,
        color = color,
        fs = widget.font_size,
        label = html_escape(&widget.label),
        label_attr = html_escape(&widget.label),
        id = id,
        var = var_attr,
        var_access = var_access,
        toggle = toggle_expr,
        type_code = type_code,
        z = z_index_style,
    )
}

/// Generate HTML for a slider widget with range input.
pub fn render_slider_html(widget: &HmiWidget, var_access: &str) -> String {
    let color = safe_color(&widget.color, "#22c55e");
    let z_index_style = widget.z_index.map_or(String::new(), |z| format!("z-index:{};", z));
    let type_code = var_type_code(widget.data_type.as_deref());
    format!(
        r#"<div class="widget slider" style="left:{x}px;top:{y}px;width:{w}px;height:{h}px;position:absolute;{z}display:flex;flex-direction:column;align-items:center;justify-content:center;padding:8px;background:#2a2a3a;border-radius:4px" {var}>
    <span style="font-size:11px;color:#999;margin-bottom:4px" data-i18n="{label_attr}">{label}</span>
    <input type="range" id="sl-in-{id}" min="{min}" max="{max}" step="any" value="{min}"
           style="width:90%;accent-color:{color}"
           oninput="sendForceVar('{vname}',{type_code},this.value)">
    <span class="widget-value" id="sl-val-{id}" style="font-size:{fs}px;color:{color}">--</span>
</div>"#,
        x = widget.x,
        y = widget.y,
        w = widget.width,
        h = widget.height,
        color = color,
        fs = widget.font_size,
        label = html_escape(&widget.label),
        label_attr = html_escape(&widget.label),
        min = js_num(widget.min, 0.0),
        max = js_num(widget.max, 100.0),
        var = var_access,
        vname = attr_js(&widget.variable_name),
        type_code = type_code,
        z = z_index_style,
        id = safe_id(&widget.id),
    )
}

/// Generate JS binding for slider value display.
/// Every slider widget registers its own binding against its own element ids,
/// so all sliders bound to one variable follow the PLC.
pub fn render_slider_js(widget: &HmiWidget) -> String {
    let var = js_escape(&widget.variable_name);
    let id = safe_id(&widget.id);
    format!(
        r#"(function() {{
    var input = document.getElementById('sl-in-{id}');
    var label = document.getElementById('sl-val-{id}');
    // The thumb follows the PLC except during an operator drag. A focus check
    // is not enough: the range keeps focus after the first touch.
    var dragging = false;
    var stopDrag = function() {{ dragging = false; }};
    if (input) {{
        input.addEventListener('pointerdown', function() {{ dragging = true; }});
        input.addEventListener('change', stopDrag);
        window.addEventListener('pointerup', stopDrag);
        window.addEventListener('pointercancel', stopDrag);
    }}
    bindings["{var}"] = bindings["{var}"] || [];
    bindings["{var}"].push(function(v) {{
        if (label) label.textContent = fmtNum(v, '{mode}');
        if (input && typeof v === 'number' && !dragging) input.value = v;
    }});
}})();"#,
        var = var,
        id = id,
        mode = num_mode(widget.data_type.as_deref()),
    )
}

/// Generate HTML for a text widget.
pub fn render_text_html(widget: &HmiWidget, var_access: &str) -> String {
    let color = safe_color(&widget.color, "#22c55e");
    let display_text = if widget.variable_name.is_empty() {
        html_escape(&widget.label)
    } else {
        "--".to_string()
    };
    let z_index_style = widget.z_index.map_or(String::new(), |z| format!("z-index:{};", z));
    format!(
        r#"<div class="widget text" style="left:{x}px;top:{y}px;width:{w}px;height:{h}px;position:absolute;{z}display:flex;align-items:center;justify-content:center" {var}>
    <span class="widget-value" id="txt-{id}" style="font-size:{fs}px;color:{color}" data-i18n="{label_attr}">{text}</span>
</div>"#,
        x = widget.x,
        y = widget.y,
        w = widget.width,
        h = widget.height,
        color = color,
        fs = widget.font_size,
        var = var_access,
        text = display_text,
        label_attr = html_escape(&widget.label),
        z = z_index_style,
        id = safe_id(&widget.id),
    )
}

/// Generate JS binding for text value updates.
pub fn render_text_js(widget: &HmiWidget) -> String {
    let var = js_escape(&widget.variable_name);
    let id = safe_id(&widget.id);
    format!(
        r#"bindings["{var}"] = bindings["{var}"] || [];
bindings["{var}"].push(function(v) {{
    var el = document.getElementById('txt-{id}');
    if (el) el.textContent = fmtNum(v, '{mode}');
}});"#,
        var = var,
        id = id,
        mode = num_mode(widget.data_type.as_deref()),
    )
}

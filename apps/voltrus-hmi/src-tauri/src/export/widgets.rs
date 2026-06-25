use crate::*;
use super::{html_escape, js_escape};

/// Generate HTML for a gauge widget using inline SVG with semicircular arc.
pub fn render_gauge_html(widget: &HmiWidget, var_access: &str) -> String {
    let cx = 100.0;
    let cy = 110.0;
    let radius = 80.0;

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
    s.push_str(&widget.id);
    s.push_str("\" d=\"");
    s.push_str(&bg_arc);
    s.push_str("\" fill=\"none\" stroke=\"");
    s.push_str(&html_escape(&widget.color));
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
    s.push_str(&widget.id);
    s.push_str("\" fill=\"");
    s.push_str(&html_escape(&widget.color));
    s.push_str("\" font-size=\"28\" font-weight=\"bold\">--</text>\n");

    s.push_str("    </svg>\n");
    s.push_str("    <span style=\"font-size:11px;color:#999\">");
    s.push_str(&html_escape(&widget.label));
    s.push_str("</span>\n");

    s.push_str("    <span style=\"font-size:10px;color:#666\">");
    s.push_str(&format!("{} - {}", widget.min, widget.max));
    s.push_str("</span>\n");

    s.push_str("</div>");
    s
}

/// Generate JS binding for gauge value updates.
pub fn render_gauge_js(widget: &HmiWidget) -> String {
    let var = js_escape(&widget.variable_name);
    let id = &widget.id;
    let radius = 80.0;
    let dash_total = std::f64::consts::PI * radius;
    let min = widget.min;
    let max = widget.max;

    format!(
        r#"bindings["{var}"] = bindings["{var}"] || [];
bindings["{var}"].push(function(v) {{
    var el = document.querySelector('[data-bind="{var}"] #gauge-val-{id}');
    var fill = document.getElementById('gauge-fill-{id}');
    if (el) el.textContent = typeof v === 'number' ? v.toFixed(1) : v;
    if (fill && typeof v === 'number') {{
        var pct = Math.max(0, Math.min(1, (v - {min}) / ({max} - {min})));
        var dashTotal = {dash_total};
        fill.style.strokeDashoffset = dashTotal * (1 - pct);
        if (pct < 0.8) fill.style.stroke = '#22c55e';
        else if (pct < 0.95) fill.style.stroke = '#eab308';
        else fill.style.stroke = '#ef4444';
    }}
}});"#
    )
}

/// Generate HTML for a chart widget using inline SVG polyline.
pub fn render_chart_html(widget: &HmiWidget, var_access: &str) -> String {
    let z_index_style = widget.z_index.map_or(String::new(), |z| format!("z-index:{};", z));
    format!(
        r#"<div class="widget chart" style="left:{x}px;top:{y}px;width:{w}px;height:{h}px;position:absolute;{z}background:#2a2a3a;border:1px solid #444;border-radius:4px;display:flex;flex-direction:column;align-items:center;padding:4px" {var}>
    <span style="font-size:11px;color:#999;flex-shrink:0">{label}</span>
    <svg id="chart-svg-{id}" viewBox="0 0 200 80" preserveAspectRatio="none" style="width:100%;flex:1;min-height:0">
        <polyline id="chart-line-{id}" fill="none" stroke="{color}" stroke-width="1.5" points=""/>
    </svg>
</div>"#,
        x = widget.x,
        y = widget.y,
        w = widget.width,
        h = widget.height,
        color = html_escape(&widget.color),
        var = var_access,
        id = widget.id,
        label = html_escape(&widget.label),
        z = z_index_style,
    )
}

/// Generate JS binding for chart value updates (keeps last 50 values).
pub fn render_chart_js(widget: &HmiWidget) -> String {
    let var = js_escape(&widget.variable_name);
    let id = &widget.id;

    format!(
        r#"if (!chartData["{var}"]) chartData["{var}"] = [];
bindings["{var}"] = bindings["{var}"] || [];
bindings["{var}"].push(function(v) {{
    if (typeof v !== 'number') return;
    chartData["{var}"].push(v);
    if (chartData["{var}"].length > 50) chartData["{var}"].shift();
    var data = chartData["{var}"];
    var line = document.getElementById('chart-line-{id}');
    if (!line || data.length < 2) return;
    var minV = Math.min.apply(null, data);
    var maxV = Math.max.apply(null, data);
    var range = maxV - minV || 1;
    var pts = [];
    for (var i = 0; i < data.length; i++) {{
        var px = (i / 49) * 200;
        var py = 80 - ((data[i] - minV) / range) * 76 - 2;
        pts.push(px.toFixed(1) + ',' + py.toFixed(1));
    }}
    line.setAttribute('points', pts.join(' '));
}});"#
    )
}

/// Generate HTML for an indicator widget (circle with glow when ON).
pub fn render_indicator_html(widget: &HmiWidget, var_access: &str) -> String {
    let z_index_style = widget.z_index.map_or(String::new(), |z| format!("z-index:{};", z));
    format!(
        r#"<div class="widget indicator" style="left:{x}px;top:{y}px;width:{w}px;height:{h}px;position:absolute;{z}display:flex;flex-direction:column;align-items:center;justify-content:center" {var}>
    <div class="indicator-light" id="ind-{id}" style="width:60%;aspect-ratio:1;border-radius:50%;background:#333;border:2px solid {color};transition:all 0.3s;box-shadow:none"></div>
    <span style="font-size:{fs}px;color:#ccc;margin-top:4px">{label}</span>
</div>"#,
        x = widget.x,
        y = widget.y,
        w = widget.width,
        h = widget.height,
        color = html_escape(&widget.color),
        var = var_access,
        id = widget.id,
        fs = widget.font_size,
        label = html_escape(&widget.label),
        z = z_index_style,
    )
}

/// Generate JS binding for indicator value updates.
pub fn render_indicator_js(widget: &HmiWidget) -> String {
    let var = js_escape(&widget.variable_name);
    let id = &widget.id;
    let color = js_escape(&widget.color);

    format!(
        r#"bindings["{var}"] = bindings["{var}"] || [];
bindings["{var}"].push(function(v) {{
    var el = document.getElementById('ind-{id}');
    if (el) {{
        var on = v === true || v === 1 || v === '1' || v === 'true';
        el.style.background = on ? '{color}' : '#333';
        el.style.boxShadow = on ? '0 0 12px 4px {color}80' : 'none';
    }}
}});"#
    )
}

/// Generate HTML for a button widget with toggle support.
pub fn render_button_html(widget: &HmiWidget) -> String {
    let var = js_escape(&widget.variable_name);
    let toggle_expr = if var.is_empty() {
        "1".to_string()
    } else {
        format!("(buttonStates['{var}'] ? 0 : 1)")
    };
    let z_index_style = widget.z_index.map_or(String::new(), |z| format!("z-index:{};", z));

    format!(
        r#"<div class="widget button" style="left:{x}px;top:{y}px;width:{w}px;height:{h}px;position:absolute;{z}">
    <button id="btn-{id}" onclick="handleButton('{var}',{toggle})" style="width:100%;height:100%;background:#2a2a3a;color:{color};border:2px solid {color};border-radius:6px;font-size:{fs}px;font-weight:600;cursor:pointer;transition:all 0.2s">{label}</button>
</div>"#,
        x = widget.x,
        y = widget.y,
        w = widget.width,
        h = widget.height,
        color = html_escape(&widget.color),
        fs = widget.font_size,
        label = html_escape(&widget.label),
        id = widget.id,
        var = var,
        toggle = toggle_expr,
        z = z_index_style,
    )
}

/// Generate HTML for a slider widget with range input.
pub fn render_slider_html(widget: &HmiWidget, var_access: &str) -> String {
    let z_index_style = widget.z_index.map_or(String::new(), |z| format!("z-index:{};", z));
    format!(
        r#"<div class="widget slider" style="left:{x}px;top:{y}px;width:{w}px;height:{h}px;position:absolute;{z}display:flex;flex-direction:column;align-items:center;justify-content:center;padding:8px;background:#2a2a3a;border-radius:4px" {var}>
    <span style="font-size:11px;color:#999;margin-bottom:4px">{label}</span>
    <input type="range" min="{min}" max="{max}" step="any" value="{min}"
           style="width:90%;accent-color:{color}"
           oninput="sendForceVar('{vname}',0,this.value)">
    <span class="widget-value" style="font-size:{fs}px;color:{color}">--</span>
</div>"#,
        x = widget.x,
        y = widget.y,
        w = widget.width,
        h = widget.height,
        color = html_escape(&widget.color),
        fs = widget.font_size,
        label = html_escape(&widget.label),
        min = widget.min,
        max = widget.max,
        var = var_access,
        vname = js_escape(&widget.variable_name),
        z = z_index_style,
    )
}

/// Generate JS binding for slider value display.
pub fn render_slider_js(widget: &HmiWidget) -> String {
    let var = js_escape(&widget.variable_name);
    format!(
        r#"bindings["{var}"] = bindings["{var}"] || [];
bindings["{var}"].push(function(v) {{
    var el = document.querySelector('[data-bind="{var}"] .widget-value');
    if (el) el.textContent = typeof v === 'number' ? v.toFixed(1) : v;
}});"#
    )
}

/// Generate HTML for a text widget.
pub fn render_text_html(widget: &HmiWidget, var_access: &str) -> String {
    let display_text = if widget.variable_name.is_empty() {
        html_escape(&widget.label)
    } else {
        "--".to_string()
    };
    let z_index_style = widget.z_index.map_or(String::new(), |z| format!("z-index:{};", z));
    format!(
        r#"<div class="widget text" style="left:{x}px;top:{y}px;width:{w}px;height:{h}px;position:absolute;{z}display:flex;align-items:center;justify-content:center" {var}>
    <span class="widget-value" style="font-size:{fs}px;color:{color}">{text}</span>
</div>"#,
        x = widget.x,
        y = widget.y,
        w = widget.width,
        h = widget.height,
        color = html_escape(&widget.color),
        fs = widget.font_size,
        var = var_access,
        text = display_text,
        z = z_index_style,
    )
}

/// Generate JS binding for text value updates.
pub fn render_text_js(widget: &HmiWidget) -> String {
    let var = js_escape(&widget.variable_name);
    format!(
        r#"bindings["{var}"] = bindings["{var}"] || [];
bindings["{var}"].push(function(v) {{
    var el = document.querySelector('[data-bind="{var}"] .widget-value');
    if (el) el.textContent = typeof v === 'number' ? v.toFixed(1) : String(v);
}});"#
    )
}

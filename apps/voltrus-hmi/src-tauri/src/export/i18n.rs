use super::{html_escape, js_escape, json_for_script};
use crate::*;

/// Generate i18n JS — translation loader, language switcher, data-i18n attribute handler.
pub fn render_i18n_js(translations: &TranslationSet, default_language: &str) -> String {
    let translations_json = json_for_script(
        &serde_json::to_string(&translations.languages).unwrap_or_else(|_| "{}".to_string()),
    );
    let default_language = js_escape(default_language);

    format!(
        r#"
// --- i18n ---
var TRANSLATIONS = {translations_json};
var currentLang = "{default_language}";

// data-i18n attributes hold the html-escaped source text (they are written
// into HTML attributes); translation keys are the raw source text, so try the
// unescaped form as well before falling back.
function htmlUnescape(s) {{
    return String(s)
        .replace(/&lt;/g, '<')
        .replace(/&gt;/g, '>')
        .replace(/&quot;/g, '"')
        .replace(/&#39;/g, "'")
        .replace(/&amp;/g, '&');
}}

function t(key) {{
    var langMap = TRANSLATIONS[currentLang];
    if (langMap && langMap[key]) return langMap[key];
    var unescaped = htmlUnescape(key);
    if (langMap && langMap[unescaped]) return langMap[unescaped];
    return unescaped; // fallback to original
}}

function switchLanguage(code) {{
    currentLang = code;
    // Update all elements with data-i18n attribute (nav buttons, widget labels)
    var elements = document.querySelectorAll('[data-i18n]');
    for (var i = 0; i < elements.length; i++) {{
        var key = elements[i].getAttribute('data-i18n');
        elements[i].textContent = t(key);
    }}
    // Update language selector
    var selector = document.getElementById('lang-select');
    if (selector) selector.value = code;
}}

// Apply the default language's translations on initial load.
switchLanguage(currentLang);
"#
    )
}

/// Generate language dropdown HTML for nav bar.
pub fn render_language_dropdown_html(
    translations: &TranslationSet,
    default_language: &str,
) -> String {
    let languages: Vec<String> = translations.languages.keys().cloned().collect();
    if languages.len() <= 1 {
        return String::new();
    }

    let options: Vec<String> = languages
        .iter()
        .map(|code| {
            let selected = if code == default_language {
                " selected"
            } else {
                ""
            };
            let display = html_escape(&code.to_uppercase());
            format!(
                "<option value=\"{}\"{}>{}</option>",
                display, selected, display
            )
        })
        .collect();

    format!(
        r#"<select id="lang-select" onchange="switchLanguage(this.value)" style="background:#2a2a3a;color:#ccc;border:1px solid #444;border-radius:3px;padding:2px 4px;font-size:11px;margin-left:auto">{}</select>"#,
        options.join("")
    )
}

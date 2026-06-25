use crate::*;

/// Generate i18n JS — translation loader, language switcher, data-i18n attribute handler.
pub fn render_i18n_js(translations: &TranslationSet, default_language: &str) -> String {
    let translations_json = serde_json::to_string(&translations.languages)
        .unwrap_or_else(|_| "{}".to_string());

    format!(
        r#"
// --- i18n ---
var TRANSLATIONS = {translations_json};
var currentLang = "{default_language}";

function t(key) {{
    var langMap = TRANSLATIONS[currentLang];
    if (langMap && langMap[key]) return langMap[key];
    return key; // fallback to original
}}

function switchLanguage(code) {{
    currentLang = code;
    // Update all elements with data-i18n attribute
    var elements = document.querySelectorAll('[data-i18n]');
    for (var i = 0; i < elements.length; i++) {{
        var key = elements[i].getAttribute('data-i18n');
        elements[i].textContent = t(key);
    }}
    // Update nav buttons
    var navBtns = document.querySelectorAll('.nav-btn');
    for (var i = 0; i < navBtns.length; i++) {{
        var key = navBtns[i].getAttribute('data-i18n');
        if (key) navBtns[i].textContent = t(key);
    }}
    // Update language selector
    var selector = document.getElementById('lang-select');
    if (selector) selector.value = code;
}}
"#
    )
}

/// Generate language dropdown HTML for nav bar.
pub fn render_language_dropdown_html(translations: &TranslationSet, default_language: &str) -> String {
    let languages: Vec<String> = translations.languages.keys().cloned().collect();
    if languages.len() <= 1 {
        return String::new();
    }

    let options: Vec<String> = languages
        .iter()
        .map(|code| {
            let selected = if code == default_language { " selected" } else { "" };
            format!("<option value=\"{}\"{}>{}</option>", code.to_uppercase(), selected, code.to_uppercase())
        })
        .collect();

    format!(
        r#"<select id="lang-select" onchange="switchLanguage(this.value)" style="background:#2a2a3a;color:#ccc;border:1px solid #444;border-radius:3px;padding:2px 4px;font-size:11px;margin-left:auto">{}</select>"#,
        options.join("")
    )
}

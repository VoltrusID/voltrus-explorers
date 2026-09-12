use mt_enip::{browse_tags, read_templates, udt_templates_to_st, CipTagDef, CipValue, EnipConfig};

/// One decoded UDT member for the frontend (IEC type name).
#[derive(serde::Serialize, Clone)]
pub struct EipImportMember {
    pub name: String,
    #[serde(rename = "type")]
    pub member_type: String,
}

/// One browsed UDT tag with its resolved template members.
#[derive(serde::Serialize, Clone)]
pub struct EipImportTag {
    pub name: String,
    /// Template (UDT type) name this tag instantiates.
    pub template: String,
    pub members: Vec<EipImportMember>,
}

/// Result of a successful UDT import browse.
#[derive(serde::Serialize)]
pub struct EipImportResult {
    pub tags: Vec<EipImportTag>,
    /// Ready-to-write `eip_types.st` content for the matched tags.
    pub generated_st: String,
}

/// Browse a controller for UDT tags, resolve their CIP Template member layouts,
/// and generate IEC 61131-3 TYPE blocks (issue PLC-7). Stateless one-shot: the
/// connection lives only for this call, independent of the I/O driver.
#[tauri::command]
pub async fn eip_import_udts(
    host: String,
    port: u16,
    tag_glob: String,
) -> Result<EipImportResult, String> {
    let config = EnipConfig {
        id: "eip-import".into(),
        host: host.clone(),
        port,
        poll_interval_ms: 1000,
    };

    let tags = browse_tags(&config)
        .await
        .map_err(|e| format!("Browse failed on {host}:{port}: {e}"))?;
    if tags.is_empty() {
        return Err(format!(
            "No tags returned by {host}:{port} — check the controller is reachable and speaks EtherNet/IP (default port 44818)."
        ));
    }

    let udt_count = tags.iter().filter(|t| t.template_id.is_some()).count();
    let matched: Vec<_> = tags
        .iter()
        .filter(|t| t.template_id.is_some() && glob_match(&tag_glob, &t.name))
        .collect();
    if matched.is_empty() {
        return Err(format!(
            "No UDT tags match glob '{tag_glob}' (controller exposes {} tag(s), {udt_count} structure tag(s)).",
            tags.len()
        ));
    }

    let mut template_ids: Vec<u16> = matched.iter().filter_map(|t| t.template_id).collect();
    template_ids.sort_unstable();
    template_ids.dedup();
    let templates = read_templates(&config, &template_ids)
        .await
        .map_err(|e| format!("Template (class 0x6C) read failed on {host}:{port}: {e}"))?;
    if templates.is_empty() {
        return Err(format!(
            "Controller {host}:{port} exposes no readable CIP Template objects; cannot resolve UDT members."
        ));
    }

    let mut import_tags = Vec::with_capacity(matched.len());
    let mut tag_defs = Vec::with_capacity(matched.len());
    for t in &matched {
        let tid = t.template_id.expect("matched filters on template_id");
        let tmpl = templates.iter().find(|tp| tp.id == tid).ok_or_else(|| {
            format!("Tag '{}' references template {tid} but the controller did not serve it.", t.name)
        })?;
        import_tags.push(EipImportTag {
            name: t.name.clone(),
            template: tmpl.name.clone(),
            members: tmpl
                .members
                .iter()
                .map(|m| EipImportMember {
                    name: m.name.clone(),
                    member_type: mt_enip::member_type_name(m.data_type).to_string(),
                })
                .collect(),
        });
        tag_defs.push(CipTagDef {
            name: t.name.clone(),
            data_type: t.data_type,
            element_count: t.element_count,
            initial_value: CipValue::default(),
            template_id: t.template_id,
        });
    }

    Ok(EipImportResult {
        tags: import_tags,
        generated_st: udt_templates_to_st(&templates, &tag_defs),
    })
}

/// Match `pattern` against `name` with `*` (any run, incl. empty) and `?`
/// (exactly one char). Case-sensitive, byte-wise — CIP tag names are
/// preserved-case identifiers.
fn glob_match(pattern: &str, name: &str) -> bool {
    let (p, n) = (pattern.as_bytes(), name.as_bytes());
    let (mut pi, mut ni) = (0usize, 0usize);
    let (mut star, mut mark) = (usize::MAX, 0usize);
    while ni < n.len() {
        if pi < p.len() && (p[pi] == b'?' || p[pi] == n[ni]) {
            pi += 1;
            ni += 1;
        } else if pi < p.len() && p[pi] == b'*' {
            star = pi;
            mark = ni;
            pi += 1;
        } else if star != usize::MAX {
            pi = star + 1;
            mark += 1;
            ni = mark;
        } else {
            return false;
        }
    }
    while pi < p.len() && p[pi] == b'*' {
        pi += 1;
    }
    pi == p.len()
}

#[cfg(test)]
mod tests {
    use super::glob_match;

    #[test]
    fn glob_star_matches_any_run() {
        assert!(glob_match("*", "Motor1"));
        assert!(glob_match("Motor*", "Motor1"));
        assert!(glob_match("*1", "Motor1"));
        assert!(glob_match("Mo*1", "Motor1"));
        assert!(glob_match("M*r1", "Motor1"));
        assert!(glob_match("", ""));
        assert!(!glob_match("", "x"));
        assert!(!glob_match("Motor", "Motor1"));
        assert!(!glob_match("motor*", "Motor1"), "case-sensitive");
    }

    #[test]
    fn glob_question_matches_single_char() {
        assert!(glob_match("Motor?", "Motor1"));
        assert!(!glob_match("Motor?", "Motor"));
        assert!(!glob_match("Motor??", "Motor1"));
        assert!(glob_match("M?tor?", "Motor1"));
    }

    #[test]
    fn glob_structure_paths() {
        assert!(glob_match("Program:Main.*", "Program:Main.Motor1"));
        assert!(glob_match("Program:*.DriveInfo", "Program:Main.Motor1.DriveInfo"));
        assert!(!glob_match("Program:*.DriveInfo", "Program:Main.Motor1.Amps"));
    }
}

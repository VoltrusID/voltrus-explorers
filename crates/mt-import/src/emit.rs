//! Shared ST source assembly: struct type bookkeeping and POU file layout.

/// A user-defined struct type ready to be emitted as a `TYPE ... END_TYPE`
/// block. Field types are IEC ST type texts (elementary names, names of other
/// struct types in this set, or function block names for instances).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StructType {
    pub name: String,
    /// `(field name, IEC type text)` in declaration order.
    pub fields: Vec<(String, String)>,
}

/// Which `VAR` block a declaration belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VarKind {
    Var,
    VarInput,
    VarOutput,
    VarInOut,
}

impl VarKind {
    fn keyword(self) -> &'static str {
        match self {
            VarKind::Var => "VAR",
            VarKind::VarInput => "VAR_INPUT",
            VarKind::VarOutput => "VAR_OUTPUT",
            VarKind::VarInOut => "VAR_IN_OUT",
        }
    }
}

/// One variable declaration line, e.g. `MotorStart : BOOL;`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Decl {
    pub text: String,
}

impl Decl {
    pub fn new(name: &str, type_text: &str, init: Option<&str>) -> Decl {
        let mut text = format!("{name} : {type_text}");
        if let Some(init) = init {
            text.push_str(" := ");
            text.push_str(init);
        }
        text.push(';');
        Decl { text }
    }
}

/// One `.st` file candidate: a single POU plus the struct types it needs.
#[derive(Clone, Debug)]
pub struct PouFile {
    pub name: String,
    pub pou_kind: &'static str,
    pub pou_name: String,
    /// Var groups in emission order (input, output, in-out, local).
    pub groups: Vec<(VarKind, Vec<Decl>)>,
    pub body: String,
    pub types: Vec<StructType>,
}

impl PouFile {
    /// Render the ST source. `TYPES` precede the POU; empty var groups are
    /// omitted so generated files stay readable.
    pub fn render(&self) -> String {
        let mut out = String::new();
        for t in &self.types {
            out.push_str("TYPE ");
            out.push_str(&t.name);
            out.push_str(" :\nSTRUCT\n");
            for (field, fty) in &t.fields {
                out.push_str("    ");
                out.push_str(field);
                out.push_str(" : ");
                out.push_str(fty);
                out.push_str(";\n");
            }
            out.push_str("END_STRUCT;\nEND_TYPE\n\n");
        }
        out.push_str(self.pou_kind);
        out.push(' ');
        out.push_str(&self.pou_name);
        out.push('\n');
        for (kind, decls) in &self.groups {
            if decls.is_empty() {
                continue;
            }
            out.push_str(kind.keyword());
            out.push('\n');
            for d in decls {
                out.push_str("    ");
                out.push_str(&d.text);
                out.push('\n');
            }
            out.push_str("END_VAR\n");
        }
        out.push_str(&self.body);
        if !self.body.is_empty() && !self.body.ends_with('\n') {
            out.push('\n');
        }
        out.push_str("END_");
        out.push_str(self.pou_kind);
        out.push('\n');
        out
    }
}

/// Pick the struct types (in their definition order) transitively reachable
/// from `root_type_names`. Names are compared case-insensitively because the
/// IEC ST lexer is case-insensitive.
pub fn reachable_structs(root_type_names: &[String], defs: &[StructType]) -> Vec<StructType> {
    let mut picked_idx: Vec<usize> = Vec::new();
    let mut work: Vec<String> = root_type_names.to_vec();

    while let Some(name) = work.pop() {
        let Some(idx) = defs.iter().position(|d| d.name.eq_ignore_ascii_case(&name)) else {
            continue;
        };
        if picked_idx.contains(&idx) {
            continue;
        }
        for (_, fty) in &defs[idx].fields {
            work.push(base_type_name(fty));
        }
        picked_idx.push(idx);
    }

    picked_idx.sort_unstable();
    picked_idx.iter().map(|&i| defs[i].clone()).collect()
}

/// The name a field type text refers to, for reachability purposes.
/// Function block instances (`TON`) and elementary fields return themselves;
/// callers only ever match against struct names.
fn base_type_name(type_text: &str) -> String {
    type_text
        .split_whitespace()
        .last()
        .unwrap_or(type_text)
        .trim_end_matches(';')
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn recipe() -> StructType {
        StructType {
            name: "Recipe".into(),
            fields: vec![
                ("target".into(), "REAL".into()),
                ("enabled".into(), "BOOL".into()),
                ("label".into(), "STRING".into()),
            ],
        }
    }

    fn nested() -> StructType {
        StructType {
            name: "Machine".into(),
            fields: vec![
                ("cfg".into(), "Recipe".into()),
                ("steps".into(), "DINT".into()),
            ],
        }
    }

    #[test]
    fn renders_type_then_pou() {
        let file = PouFile {
            name: "Main.st".into(),
            pou_kind: "PROGRAM",
            pou_name: "Main".into(),
            groups: vec![(
                VarKind::VarInput,
                vec![Decl::new("start_cmd", "BOOL", Some("FALSE"))],
            )],
            body: "running := start_cmd;\n".into(),
            types: vec![recipe()],
        };
        let src = file.render();
        assert!(src.contains("TYPE Recipe :"), "{src}");
        assert!(src.contains("STRUCT"), "{src}");
        assert!(src.contains("    target : REAL;"), "{src}");
        assert!(src.contains("PROGRAM Main"), "{src}");
        assert!(src.contains("VAR_INPUT"), "{src}");
        assert!(src.contains("start_cmd : BOOL := FALSE;"), "{src}");
        assert!(src.trim_end().ends_with("END_PROGRAM"));
        // TYPE block must come before the POU.
        let type_pos = src.find("TYPE Recipe").unwrap();
        let pou_pos = src.find("PROGRAM Main").unwrap();
        assert!(type_pos < pou_pos);
    }

    #[test]
    fn empty_groups_are_omitted() {
        let file = PouFile {
            name: "T.st".into(),
            pou_kind: "PROGRAM",
            pou_name: "T".into(),
            groups: vec![(VarKind::VarInOut, vec![])],
            body: String::new(),
            types: vec![],
        };
        let src = file.render();
        assert!(!src.contains("VAR_IN_OUT"), "{src}");
        assert!(src.contains("PROGRAM T\n"), "{src}");
    }

    #[test]
    fn reachability_pulls_transitive_structs_in_definition_order() {
        let defs = vec![recipe(), nested()];
        let picked = reachable_structs(&["machine".to_string()], &defs);
        // "machine" (lowercase) matches "Machine"; its field pulls "Recipe".
        assert_eq!(picked.len(), 2);
        assert_eq!(picked[0].name, "Recipe");
        assert_eq!(picked[1].name, "Machine");
    }

    #[test]
    fn reachability_ignores_unknown_names() {
        let defs = vec![recipe()];
        assert!(reachable_structs(&["Nope".to_string()], &defs).is_empty());
        assert!(reachable_structs(&[], &defs).is_empty());
    }

    #[test]
    fn decl_formats_with_and_without_init() {
        assert_eq!(Decl::new("x", "BOOL", None).text, "x : BOOL;");
        assert_eq!(Decl::new("x", "DINT", Some("0")).text, "x : DINT := 0;");
    }
}

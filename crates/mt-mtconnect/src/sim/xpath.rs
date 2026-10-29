//! The XPath subset the simulator understands for the `path` query parameter.
//!
//! Grammar: `expr ('|' expr)*`, where each expr is one or more steps `('/' | '//') name
//! ('[' '@' attr ('=' quoted)? ']')*`; `name` is an element name or `*`. Paths are evaluated
//! against `Devices/Device/<components...>/DataItem`; a data item is selected when it is, or
//! lies below, a node matched by the expression.

#[derive(Debug, Clone)]
pub(crate) struct Node {
    pub name: String,
    pub attrs: Vec<(String, String)>,
}

#[derive(Debug, Clone)]
struct Predicate {
    attr: String,
    value: Option<String>,
}

#[derive(Debug, Clone)]
struct Step {
    descendant: bool,
    name: String,
    predicates: Vec<Predicate>,
}

#[derive(Debug, Clone)]
pub(crate) struct Selector {
    alternatives: Vec<Vec<Step>>,
}

impl Selector {
    pub fn parse(expr: &str) -> Result<Self, String> {
        let mut alternatives = Vec::new();
        for alt in split_top_level(expr, '|') {
            alternatives.push(parse_steps(alt.trim())?);
        }
        if alternatives.is_empty() {
            return Err("empty path".into());
        }
        Ok(Self { alternatives })
    }

    /// True if any node of `chain` (root first, the data item last) is selected.
    pub fn selects(&self, chain: &[Node]) -> bool {
        self.alternatives.iter().any(|steps| match_from(steps, 0, chain, None))
    }
}

fn split_top_level(s: &str, sep: char) -> Vec<&str> {
    let mut out = Vec::new();
    let mut depth = 0i32;
    let mut quote: Option<char> = None;
    let mut start = 0;
    for (i, c) in s.char_indices() {
        match quote {
            Some(q) if c == q => quote = None,
            Some(_) => {}
            None => match c {
                '"' | '\'' => quote = Some(c),
                '[' => depth += 1,
                ']' => depth -= 1,
                c if c == sep && depth == 0 => {
                    out.push(&s[start..i]);
                    start = i + c.len_utf8();
                }
                _ => {}
            },
        }
    }
    out.push(&s[start..]);
    out
}

fn parse_steps(expr: &str) -> Result<Vec<Step>, String> {
    if !expr.starts_with('/') {
        return Err(format!("path '{expr}' must start with '/' or '//'"));
    }
    let chars: Vec<char> = expr.chars().collect();
    let mut i = 0;
    let mut steps = Vec::new();
    while i < chars.len() {
        if chars[i] != '/' {
            return Err(format!("unexpected '{}' in path '{expr}'", chars[i]));
        }
        i += 1;
        let descendant = i < chars.len() && chars[i] == '/';
        if descendant {
            i += 1;
        }
        let name_start = i;
        while i < chars.len() && !matches!(chars[i], '/' | '[') {
            i += 1;
        }
        let name: String = chars[name_start..i].iter().collect::<String>().trim().to_string();
        if name.is_empty() {
            return Err(format!("missing element name in path '{expr}'"));
        }
        let mut predicates = Vec::new();
        while i < chars.len() && chars[i] == '[' {
            let close = chars[i..]
                .iter()
                .position(|c| *c == ']')
                .ok_or_else(|| format!("unterminated predicate in path '{expr}'"))?;
            let body: String = chars[i + 1..i + close].iter().collect();
            predicates.push(parse_predicate(body.trim())?);
            i += close + 1;
        }
        steps.push(Step { descendant, name, predicates });
    }
    Ok(steps)
}

fn parse_predicate(body: &str) -> Result<Predicate, String> {
    let rest = body
        .strip_prefix('@')
        .ok_or_else(|| format!("unsupported predicate '[{body}]' (only [@attr] and [@attr=\"value\"])"))?;
    match rest.split_once('=') {
        None => Ok(Predicate { attr: rest.trim().to_string(), value: None }),
        Some((attr, v)) => {
            let v = v.trim();
            let unquoted = v
                .strip_prefix('"')
                .and_then(|x| x.strip_suffix('"'))
                .or_else(|| v.strip_prefix('\'').and_then(|x| x.strip_suffix('\'')))
                .ok_or_else(|| format!("predicate value in '[{body}]' must be quoted"))?;
            Ok(Predicate { attr: attr.trim().to_string(), value: Some(unquoted.to_string()) })
        }
    }
}

fn node_matches(step: &Step, node: &Node) -> bool {
    (step.name == "*" || step.name == node.name)
        && step.predicates.iter().all(|p| {
            node.attrs
                .iter()
                .find(|(k, _)| *k == p.attr)
                .is_some_and(|(_, v)| p.value.as_deref().is_none_or(|want| want == v))
        })
}

fn match_from(steps: &[Step], si: usize, chain: &[Node], prev: Option<usize>) -> bool {
    let Some(step) = steps.get(si) else { return true };
    let first = prev.map_or(0, |p| p + 1);
    let candidates = if step.descendant { first..chain.len() } else { first..(first + 1).min(chain.len()) };
    candidates.into_iter().any(|ci| node_matches(step, &chain[ci]) && match_from(steps, si + 1, chain, Some(ci)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn n(name: &str, attrs: &[(&str, &str)]) -> Node {
        Node { name: name.into(), attrs: attrs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect() }
    }

    fn chain() -> Vec<Node> {
        vec![
            n("Devices", &[]),
            n("Device", &[("name", "VMC"), ("id", "vmc")]),
            n("Axes", &[("id", "axes")]),
            n("Linear", &[("id", "x"), ("name", "X")]),
            n("DataItem", &[("id", "Xpos"), ("type", "POSITION"), ("category", "SAMPLE")]),
        ]
    }

    #[test]
    fn selects_by_component_type_id_and_union() {
        let c = chain();
        assert!(Selector::parse("//Axes").unwrap().selects(&c));
        assert!(Selector::parse("//Linear").unwrap().selects(&c));
        assert!(!Selector::parse("//Controller").unwrap().selects(&c));
        assert!(Selector::parse("//Axes//DataItem[@type=\"POSITION\"]").unwrap().selects(&c));
        assert!(!Selector::parse("//Axes//DataItem[@type='LOAD']").unwrap().selects(&c));
        assert!(Selector::parse("//Controller | //DataItem[@id=\"Xpos\"]").unwrap().selects(&c));
        assert!(Selector::parse("/Devices/Device[@name=\"VMC\"]//Linear[@name]").unwrap().selects(&c));
        assert!(!Selector::parse("/Devices/Axes").unwrap().selects(&c), "child step must be adjacent");
        assert!(Selector::parse("//Axes/Linear").unwrap().selects(&c));
        assert!(Selector::parse("//*[@id=\"axes\"]").unwrap().selects(&c));
    }

    #[test]
    fn rejects_bad_paths() {
        for bad in ["Axes", "//", "//Axes[", "//Axes[@id=x]", "//Axes[text()]", ""] {
            assert!(Selector::parse(bad).is_err(), "{bad}");
        }
    }
}

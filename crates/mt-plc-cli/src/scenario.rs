use anyhow::{Context, bail};
use mt_plc_types::PlcValue;
use serde::Deserialize;
use serde_yaml::{Mapping, Number, Value};

/// Default virtual scan-cycle period in milliseconds.
pub const DEFAULT_SCAN_MS: u64 = 10;

/// Default tolerance for `Real` matchers written as bare numbers.
const DEFAULT_REAL_TOL: f64 = 1e-4;

/// One `*.test.yaml` file: a program plus named virtual-time scenarios.
#[derive(Deserialize, Debug)]
pub struct ScenarioFile {
    /// Program source file, resolved relative to this YAML file.
    pub program: String,
    /// Virtual scan-cycle period in milliseconds (default 10).
    #[serde(default)]
    pub scan_ms: Option<u64>,
    #[serde(default)]
    pub scenarios: Vec<Scenario>,
}

#[derive(Deserialize, Debug)]
pub struct Scenario {
    pub name: String,
    /// Variables seeded before the first scan; values are coerced to the
    /// declared slot types.
    #[serde(default)]
    pub given: Mapping,
    #[serde(default)]
    pub steps: Vec<Step>,
    /// Final matchers, checked after the last step.
    #[serde(default)]
    pub expect: Option<Mapping>,
}

/// One ordered step of a scenario: optional variable writes and an
/// optional virtual-time advance. A step must carry at least one of the
/// two — `run_scenario` rejects steps with neither, naming the index.
#[derive(Deserialize, Debug)]
pub struct Step {
    /// Virtual time to run: `500ms`, `2s` or `25scans`.
    pub advance: Option<String>,
    /// Variable writes applied before this step's scans, coerced to the
    /// declared slot types exactly like `given:`. A set-only step applies
    /// its writes without running any scan.
    #[serde(default)]
    pub set: Option<Mapping>,
    /// Matchers checked right after this step's scans.
    #[serde(default)]
    pub expect: Option<Mapping>,
}

/// Variable matcher: a bare YAML scalar is an exact match,
/// `{approx: x, tol: y}` accepts a numeric window.
#[derive(Debug, Clone, PartialEq)]
pub enum Matcher {
    Bool(bool),
    Int(i64),
    Real { value: f64, tol: f64 },
    Text(String),
}

impl Matcher {
    pub fn from_yaml(raw: &Value) -> anyhow::Result<Matcher> {
        match raw {
            Value::Bool(b) => Ok(Matcher::Bool(*b)),
            Value::Number(n) => {
                // Only genuine integers become Int matchers; a float like
                // 41.5 must keep its default-tolerance Real window instead
                // of being truncated by yaml_i64's float fallback.
                if let Some(i) = n
                    .as_i64()
                    .or_else(|| n.as_u64().and_then(|u| i64::try_from(u).ok()))
                {
                    Ok(Matcher::Int(i))
                } else {
                    Ok(Matcher::Real {
                        value: n.as_f64().context("matcher number is out of range")?,
                        tol: DEFAULT_REAL_TOL,
                    })
                }
            }
            Value::String(s) => Ok(Matcher::Text(s.clone())),
            Value::Mapping(m) => {
                let approx = required_number(m, "approx")?;
                let tol = match optional_number(m, "tol")? {
                    Some(t) => t,
                    None => DEFAULT_REAL_TOL,
                };
                Ok(Matcher::Real { value: approx, tol })
            }
            other => bail!("unsupported matcher: {other:?}"),
        }
    }

    pub fn matches(&self, actual: &PlcValue) -> bool {
        match self {
            Matcher::Bool(expected) => matches!(actual, PlcValue::Bool(b) if b == expected),
            Matcher::Int(expected) => numeric(actual).is_some_and(|v| v == *expected as f64),
            Matcher::Real { value, tol } => {
                numeric(actual).is_some_and(|v| (v - value).abs() <= *tol)
            }
            Matcher::Text(expected) => match actual {
                PlcValue::String(s) => s == expected,
                PlcValue::Enum(e) => e == expected,
                _ => false,
            },
        }
    }

    pub fn describe(&self) -> String {
        match self {
            Matcher::Bool(true) => "TRUE".into(),
            Matcher::Bool(false) => "FALSE".into(),
            Matcher::Int(i) => i.to_string(),
            Matcher::Real { value, tol } => format!("{value} ±{tol}"),
            Matcher::Text(s) => format!("'{s}'"),
        }
    }
}

/// Coerce a `given:` YAML scalar to the declared type of the target slot so
/// seeded values keep the program's variable semantics (e.g. `level: 40`
/// becomes `Real(40.0)`).
pub fn coerce_given(name: &str, slot: &PlcValue, raw: &Value) -> anyhow::Result<PlcValue> {
    let wrong = |want: &str| {
        anyhow::anyhow!(
            "cannot assign {} to {} variable `{}`",
            describe_yaml(raw),
            want,
            name
        )
    };
    match slot {
        PlcValue::Bool(_) => Ok(PlcValue::Bool(match raw {
            Value::Bool(b) => *b,
            Value::Number(n) => yaml_i64(n).ok_or_else(|| wrong("BOOL"))? != 0,
            _ => return Err(wrong("BOOL")),
        })),
        PlcValue::Real(_) => Ok(PlcValue::Real(match raw {
            Value::Number(n) => n.as_f64().ok_or_else(|| wrong("REAL"))? as f32,
            _ => return Err(wrong("REAL")),
        })),
        PlcValue::Byte(_) => Ok(PlcValue::Byte(
            u8::try_from(yaml_int(raw).ok_or_else(|| wrong("BYTE"))?).map_err(|_| wrong("BYTE"))?,
        )),
        PlcValue::Int(_) => Ok(PlcValue::Int(
            i16::try_from(yaml_int(raw).ok_or_else(|| wrong("INT"))?).map_err(|_| wrong("INT"))?,
        )),
        PlcValue::Dint(_) => Ok(PlcValue::Dint(
            i32::try_from(yaml_int(raw).ok_or_else(|| wrong("DINT"))?)
                .map_err(|_| wrong("DINT"))?,
        )),
        PlcValue::Word(_) => Ok(PlcValue::Word(
            u16::try_from(yaml_int(raw).ok_or_else(|| wrong("WORD"))?)
                .map_err(|_| wrong("WORD"))?,
        )),
        PlcValue::Dword(_) => Ok(PlcValue::Dword(
            u32::try_from(yaml_int(raw).ok_or_else(|| wrong("DWORD"))?)
                .map_err(|_| wrong("DWORD"))?,
        )),
        PlcValue::Time(_) => Ok(PlcValue::Time(
            i32::try_from(yaml_int(raw).ok_or_else(|| wrong("TIME"))?)
                .map_err(|_| wrong("TIME"))?,
        )),
        PlcValue::String(_) => Ok(PlcValue::String(match raw {
            Value::String(s) => s.clone(),
            _ => return Err(wrong("STRING")),
        })),
        other => bail!(
            "variable `{}` has unsupported type {} for seeding",
            name,
            other.type_name()
        ),
    }
}

/// Parse an `advance:` spec into a scan count. Duration advances are
/// floored to whole scan cycles: `99ms` at `scan_ms: 10` runs 9 scans.
pub fn parse_advance(spec: &str, scan_ms: u64) -> anyhow::Result<usize> {
    if scan_ms == 0 {
        bail!("scan_ms must be greater than zero");
    }
    let spec = spec.trim();
    let split = spec
        .find(|c: char| !c.is_ascii_digit() && c != '.')
        .context(format!(
            "invalid advance `{spec}`: missing unit (ms, s or scans)"
        ))?;
    let (num_part, unit) = spec.split_at(split);
    let unit = unit.trim().to_ascii_lowercase();
    if num_part.is_empty() {
        bail!("invalid advance `{spec}`: missing duration");
    }
    match unit.as_str() {
        "scans" => {
            let scans: u64 = num_part
                .parse()
                .map_err(|_| anyhow::anyhow!("invalid scan count `{num_part}`"))?;
            Ok(scans as usize)
        }
        "ms" | "s" => {
            let value: f64 = num_part
                .parse()
                .map_err(|_| anyhow::anyhow!("invalid duration `{num_part}`"))?;
            if value < 0.0 {
                bail!("advance must not be negative");
            }
            let ms = if unit == "s" { value * 1000.0 } else { value };
            Ok((ms / scan_ms as f64).floor() as usize)
        }
        other => bail!("invalid advance `{spec}`: unknown unit `{other}` (use ms, s or scans)"),
    }
}

/// Convert an `expect:` mapping into ordered (variable, matcher) pairs.
pub fn parse_expect(map: &Mapping) -> anyhow::Result<Vec<(String, Matcher)>> {
    map.iter()
        .map(|(key, raw)| {
            let name = key
                .as_str()
                .context("expect keys must be variable names")?
                .to_string();
            Ok((name, Matcher::from_yaml(raw)?))
        })
        .collect()
}

fn numeric(value: &PlcValue) -> Option<f64> {
    match value {
        PlcValue::Byte(v) => Some(f64::from(*v)),
        PlcValue::Int(v) => Some(f64::from(*v)),
        PlcValue::Dint(v) => Some(f64::from(*v)),
        PlcValue::Word(v) => Some(f64::from(*v)),
        PlcValue::Dword(v) => Some(f64::from(*v)),
        PlcValue::Time(v) => Some(f64::from(*v)),
        PlcValue::Real(v) => Some(f64::from(*v)),
        _ => None,
    }
}

fn yaml_i64(n: &Number) -> Option<i64> {
    if let Some(i) = n.as_i64() {
        return Some(i);
    }
    if let Some(u) = n.as_u64() {
        return i64::try_from(u).ok();
    }
    n.as_f64().map(|f| f as i64)
}

fn yaml_int(raw: &Value) -> Option<i64> {
    match raw {
        Value::Number(n) => yaml_i64(n),
        _ => None,
    }
}

fn required_number(map: &Mapping, key: &str) -> anyhow::Result<f64> {
    optional_number(map, key)?.with_context(|| format!("`{key}` is required"))
}

fn optional_number(map: &Mapping, key: &str) -> anyhow::Result<Option<f64>> {
    match map.get(&Value::from(key)) {
        None => Ok(None),
        Some(Value::Number(n)) => Ok(Some(
            n.as_f64()
                .with_context(|| format!("`{key}` is out of range"))?,
        )),
        Some(other) => bail!("`{key}` must be a number, got {other:?}"),
    }
}

fn describe_yaml(raw: &Value) -> String {
    match raw {
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => n.to_string(),
        Value::String(s) => format!("'{s}'"),
        _ => "this value".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn yaml(raw: &str) -> Value {
        serde_yaml::from_str(raw).unwrap()
    }

    #[test]
    fn parses_full_scenario_file() {
        let file: ScenarioFile = serde_yaml::from_str(
            r#"
program: Tank.st
scan_ms: 5
scenarios:
  - name: fills tank
    given:
      level: 40
    steps:
      - advance: 500ms
        expect:
          inlet_valve: true
          level: {approx: 90.0, tol: 0.5}
    expect:
      high_level_alarm: false
"#,
        )
        .unwrap();

        assert_eq!(file.program, "Tank.st");
        assert_eq!(file.scan_ms, Some(5));
        assert_eq!(file.scenarios.len(), 1);
        let scenario = &file.scenarios[0];
        assert_eq!(scenario.name, "fills tank");
        assert_eq!(scenario.given.len(), 1);
        assert_eq!(scenario.steps.len(), 1);
        assert_eq!(scenario.steps[0].advance.as_deref(), Some("500ms"));
        assert_eq!(scenario.steps[0].expect.as_ref().unwrap().len(), 2);
        assert!(scenario.expect.is_some());
    }

    #[test]
    fn scenario_file_defaults() {
        let file: ScenarioFile =
            serde_yaml::from_str("program: P.st\nscenarios:\n  - name: bare\n").unwrap();
        assert_eq!(file.scan_ms, None);
        assert!(file.scenarios[0].given.is_empty());
        assert!(file.scenarios[0].steps.is_empty());
        assert!(file.scenarios[0].expect.is_none());
    }

    #[test]
    fn set_parses_and_advance_is_optional() {
        let file: ScenarioFile = serde_yaml::from_str(
            "program: P.st\nscenarios:\n  - name: write only\n    steps:\n      - set: {level: 40}\n        expect: {level: 40}\n",
        )
        .unwrap();
        let step = &file.scenarios[0].steps[0];
        assert_eq!(step.advance, None);
        let set = step.set.as_ref().unwrap();
        assert_eq!(set.len(), 1);
        assert!(set.contains_key(&yaml("level")));
    }

    #[test]
    fn parses_advance_units() {
        assert_eq!(parse_advance("500ms", 10).unwrap(), 50);
        assert_eq!(parse_advance("2s", 10).unwrap(), 200);
        assert_eq!(parse_advance("1.5s", 10).unwrap(), 150);
        assert_eq!(parse_advance("25scans", 10).unwrap(), 25);
        assert_eq!(parse_advance("10 scans", 10).unwrap(), 10);
        assert_eq!(parse_advance("99ms", 10).unwrap(), 9, "durations floor");
        assert!(parse_advance("10", 10).is_err());
        assert!(parse_advance("s", 10).is_err());
        assert!(parse_advance("-1s", 10).is_err());
        assert!(parse_advance("5lightyears", 10).is_err());
        assert!(parse_advance("1s", 0).is_err());
    }

    #[test]
    fn matchers_match_plc_values() {
        let truthy = Matcher::from_yaml(&yaml("true")).unwrap();
        assert!(truthy.matches(&PlcValue::Bool(true)));
        assert!(!truthy.matches(&PlcValue::Bool(false)));

        let one = Matcher::from_yaml(&yaml("1")).unwrap();
        assert!(one.matches(&PlcValue::Int(1)));
        assert!(one.matches(&PlcValue::Dint(1)));
        assert!(one.matches(&PlcValue::Real(1.0)));
        assert!(!one.matches(&PlcValue::Bool(true)), "bools are not ints");

        let window = Matcher::from_yaml(&yaml("{approx: 90.0, tol: 0.5}")).unwrap();
        assert!(window.matches(&PlcValue::Real(89.8)));
        assert!(!window.matches(&PlcValue::Real(89.4)));
        assert!(!window.matches(&PlcValue::Bool(true)));

        let exact_real = Matcher::from_yaml(&yaml("90.5")).unwrap();
        assert_eq!(
            exact_real,
            Matcher::Real {
                value: 90.5,
                tol: DEFAULT_REAL_TOL
            }
        );

        let text = Matcher::from_yaml(&yaml("RUN")).unwrap();
        assert!(text.matches(&PlcValue::Enum("RUN".into())));
        assert!(!text.matches(&PlcValue::Int(0)));
    }

    #[test]
    fn approx_requires_a_number() {
        assert!(Matcher::from_yaml(&yaml("{tol: 0.5}")).is_err());
        assert!(Matcher::from_yaml(&yaml("{approx: high}")).is_err());
        assert!(Matcher::from_yaml(&yaml("[]")).is_err());
    }

    #[test]
    fn given_coerces_to_slot_type() {
        let twenty = yaml("20");
        assert_eq!(
            coerce_given("level", &PlcValue::Real(0.0), &twenty).unwrap(),
            PlcValue::Real(20.0)
        );
        assert_eq!(
            coerce_given("cycles", &PlcValue::Dint(0), &twenty).unwrap(),
            PlcValue::Dint(20)
        );
        assert_eq!(
            coerce_given("inlet", &PlcValue::Bool(false), &yaml("true")).unwrap(),
            PlcValue::Bool(true)
        );
        assert_eq!(
            coerce_given("delay", &PlcValue::Time(0), &yaml("500")).unwrap(),
            PlcValue::Time(500)
        );

        assert!(coerce_given("level", &PlcValue::Real(0.0), &yaml("high")).is_err());
        assert!(coerce_given("state", &PlcValue::Int(0), &yaml("70000")).is_err());
        assert!(coerce_given("mask", &PlcValue::Word(0), &yaml("70000")).is_err());
        assert!(coerce_given("recipe", &PlcValue::Struct(vec![]), &yaml("1")).is_err());
    }

    #[test]
    fn parse_expect_preserves_order_and_names() {
        let map: Mapping =
            serde_yaml::from_str("inlet_valve: true\nlevel: {approx: 41.0, tol: 0.001}\n").unwrap();
        let parsed = parse_expect(&map).unwrap();
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].0, "inlet_valve");
        assert_eq!(parsed[0].1, Matcher::Bool(true));
        assert_eq!(parsed[1].0, "level");
        assert!(parsed[1].1.matches(&PlcValue::Real(41.0)));
    }
}

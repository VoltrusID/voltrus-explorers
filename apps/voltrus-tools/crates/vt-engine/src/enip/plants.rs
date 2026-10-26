//! The plants the EtherNet/IP adapter simulator plays (converted from the
//! ENIP Explorer app's simulator), and a client that reads one.

use std::sync::LazyLock;

use mt_enip::SimPointDef;
use serde::Deserialize;
use vt_protocol::{CipType, EnipAddress, EnipClientSpec, EnipPoint};

const ENIP: &str = include_str!("../../templates/enip.json");

/// A plant the adapter simulator plays: its tags and how their values move.
///
/// Each point's `source` says how it moves: `wave` (the default) drifts
/// around `base`, `drive` follows the `speedRef` point while `mode` is not 0
/// (scaled by its own `base`), `counter` counts up by `variance` a second.
/// `writable` points are setpoints that hold what they are set to.
#[derive(Debug, Clone, Deserialize)]
pub struct EnipTemplate {
    pub id: String,
    pub name: String,
    pub description: String,
    pub points: Vec<SimPointDef>,
}

static TEMPLATES: LazyLock<Vec<EnipTemplate>> =
    LazyLock::new(|| match serde_json::from_str(ENIP) {
        Ok(templates) => templates,
        Err(error) => {
            tracing::error!("built-in EtherNet/IP templates are invalid: {error}");
            Vec::new()
        }
    });

/// Every built-in EtherNet/IP plant.
pub fn enip() -> &'static [EnipTemplate] {
    &TEMPLATES
}

pub fn find_enip(id: &str) -> Option<&'static EnipTemplate> {
    enip().iter().find(|template| template.id == id)
}

/// The CIP type a template point is served as.
pub fn point_type(point: &SimPointDef) -> CipType {
    if point.is_bool {
        return CipType::Bool;
    }
    match point.data_type.as_str() {
        "dint" => CipType::Dint,
        "int" => CipType::Int,
        "sint" => CipType::Sint,
        "lint" => CipType::Lint,
        "lreal" => CipType::Lreal,
        "bool" => CipType::Bool,
        _ => CipType::Real,
    }
}

/// A tag's name as people say it: without its `Program:<name>.` scope.
pub fn short_name(tag: &str) -> &str {
    match tag.strip_prefix("Program:") {
        Some(scoped) => scoped.split_once('.').map_or(scoped, |(_, name)| name),
        None => tag,
    }
}

/// Byte offset and type of each point in the adapter's input assembly
/// (instance 100): every tag's value in tag name order, as `mt-enip` lays
/// it out. Indexed like the template's points.
pub fn assembly_layout(template: &EnipTemplate) -> Vec<(usize, CipType)> {
    let mut order: Vec<usize> = (0..template.points.len()).collect();
    order.sort_by(|a, b| template.points[*a].name.cmp(&template.points[*b].name));
    let mut layout = vec![(0, CipType::Real); template.points.len()];
    let mut offset = 0;
    for index in order {
        let data_type = point_type(&template.points[index]);
        layout[index] = (offset, data_type);
        offset += data_type.size();
    }
    layout
}

/// A client of a simulated plant on this computer: each tag by name with its
/// unit, and the first measurement once more through the input assembly.
pub fn enip_client_for(template: &EnipTemplate, host: &str, port: u16) -> EnipClientSpec {
    let mut points: Vec<EnipPoint> = template
        .points
        .iter()
        .map(|point| EnipPoint {
            name: short_name(&point.name).to_owned(),
            address: EnipAddress::Tag {
                tag: point.name.clone(),
            },
            unit: point.unit.clone().unwrap_or_default(),
        })
        .collect();
    let layout = assembly_layout(template);
    if let Some((index, point)) = template
        .points
        .iter()
        .enumerate()
        .find(|(_, point)| !point.writable && !point.is_bool)
        && let Some(&(offset, data_type)) = layout.get(index)
        && let Ok(offset) = u16::try_from(offset)
    {
        points.push(EnipPoint {
            name: format!("{} (assembly)", short_name(&point.name)),
            address: EnipAddress::Attribute {
                class: mt_enip::ASSEMBLY_CLASS_ID,
                instance: mt_enip::INPUT_ASSEMBLY,
                attribute: 3,
                offset,
                data_type,
            },
            unit: point.unit.clone().unwrap_or_default(),
        });
    }
    EnipClientSpec {
        host: host.into(),
        port,
        poll_ms: 500,
        points,
        ..EnipClientSpec::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_plant_loads_with_unique_tags_and_something_to_set() {
        let templates = enip();
        assert!(templates.len() >= 3, "templates: {}", templates.len());
        for template in templates {
            let mut names: Vec<&str> = template.points.iter().map(|p| p.name.as_str()).collect();
            names.sort_unstable();
            names.dedup();
            assert_eq!(
                names.len(),
                template.points.len(),
                "{}: duplicate tag",
                template.id
            );
            for point in &template.points {
                assert!(
                    EnipAddress::parse(&point.name).is_ok(),
                    "{}: {} is not a tag name",
                    template.id,
                    point.name
                );
                assert!(
                    ["wave", "drive", "counter"].contains(&point.source.as_str()),
                    "{}: {} moves by {}",
                    template.id,
                    point.name,
                    point.source
                );
            }
            assert!(
                template.points.iter().any(|p| p.writable),
                "{}",
                template.id
            );
        }
    }

    #[test]
    fn the_assembly_follows_tag_name_order() {
        let template = EnipTemplate {
            id: "t".into(),
            name: "T".into(),
            description: String::new(),
            points: serde_json::from_str(
                r#"[{"key":"b","name":"B","data_type":"real"},
                    {"key":"a","name":"A","data_type":"bool","is_bool":true},
                    {"key":"c","name":"C","data_type":"dint"}]"#,
            )
            .expect("points"),
        };
        assert_eq!(
            assembly_layout(&template),
            [(1, CipType::Real), (0, CipType::Bool), (5, CipType::Dint)]
        );
        let client = enip_client_for(&template, "127.0.0.1", 44830);
        assert_eq!(client.points.len(), 4);
        assert_eq!(client.points[3].address.to_string(), "@0x04/100/3+1:REAL");
    }

    #[test]
    fn short_names_drop_the_program_scope() {
        assert_eq!(short_name("Program:Main.Motor1.Speed"), "Motor1.Speed");
        assert_eq!(short_name("Line_Rate"), "Line_Rate");
        assert_eq!(short_name("Program:Main"), "Main");
    }
}

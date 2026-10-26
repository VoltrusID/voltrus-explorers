//! Built-in device templates: the Modbus simulator's register maps (the same
//! ones the Modbus Poll app ships, converted from its `templates.ts`) and the
//! BACnet device simulator's plants (from the BACnet Explorer app).

use std::sync::LazyLock;

use mt_bacnet::SimPointDef;

pub use crate::enip::plants::{EnipTemplate, assembly_layout, enip, enip_client_for, find_enip};
use mt_modbus::SimDeviceProfile;
use serde::Deserialize;
use vt_protocol::{
    Area, BacnetClientSpec, DataType, Dnp3MasterSpec, Dnp3PointKind, Dnp3PointName, ModbusLink,
    ModbusMasterSpec, RegisterBlock, WordOrder,
};

const MODBUS: &str = include_str!("../templates/modbus.json");
const BACNET: &str = include_str!("../templates/bacnet.json");

pub use crate::s7::template::{S7SimPoint, S7Template, find_s7, s7, s7_client_for};

static TEMPLATES: LazyLock<Vec<SimDeviceProfile>> =
    LazyLock::new(|| match serde_json::from_str(MODBUS) {
        Ok(templates) => templates,
        Err(error) => {
            tracing::error!("built-in Modbus templates are invalid: {error}");
            Vec::new()
        }
    });

/// Every built-in Modbus device template.
pub fn modbus() -> &'static [SimDeviceProfile] {
    &TEMPLATES
}

pub fn find_modbus(id: &str) -> Option<&'static SimDeviceProfile> {
    modbus().iter().find(|profile| profile.id == id)
}

/// A plant the BACnet device simulator plays: its objects and how their
/// values move.
#[derive(Debug, Clone, Deserialize)]
pub struct BacnetTemplate {
    pub id: String,
    pub name: String,
    pub description: String,
    pub points: Vec<SimPointDef>,
}

static BACNET_TEMPLATES: LazyLock<Vec<BacnetTemplate>> =
    LazyLock::new(|| match serde_json::from_str(BACNET) {
        Ok(templates) => templates,
        Err(error) => {
            tracing::error!("built-in BACnet templates are invalid: {error}");
            Vec::new()
        }
    });

/// Every built-in BACnet plant.
pub fn bacnet() -> &'static [BacnetTemplate] {
    &BACNET_TEMPLATES
}

pub fn find_bacnet(id: &str) -> Option<&'static BacnetTemplate> {
    bacnet().iter().find(|template| template.id == id)
}

/// A client of a simulated BACnet device on this computer, subscribing to
/// its changes.
pub fn bacnet_client_for(host: &str, port: u16, device_instance: u32) -> BacnetClientSpec {
    BacnetClientSpec {
        host: host.into(),
        port,
        device_instance: Some(device_instance),
        ..BacnetClientSpec::default()
    }
}

/// Registers a template occupies after its base address: measurements are
/// float32 (2 registers), counters float64 (4 registers).
pub fn register_span(profile: &SimDeviceProfile) -> u32 {
    profile
        .registers
        .iter()
        .map(|register| if register.is_counter { 4 } else { 2 })
        .sum()
}

/// A master that polls a simulated template device the way the simulator lays
/// it out: one named block per register (holding area, big-endian) carrying
/// the register's unit. The client merges adjacent blocks into bulk reads.
pub fn master_for(
    profile: &SimDeviceProfile,
    host: &str,
    port: u16,
    unit_id: u8,
) -> ModbusMasterSpec {
    let mut blocks: Vec<RegisterBlock> = Vec::with_capacity(profile.registers.len());
    let mut address = u32::from(profile.base_address);
    for register in &profile.registers {
        let Ok(start) = u16::try_from(address) else {
            break;
        };
        blocks.push(RegisterBlock {
            name: register.name.replace('_', " "),
            area: Area::Holding,
            start,
            count: 1,
            data_type: if register.is_counter {
                DataType::F64
            } else {
                DataType::F32
            },
            unit: register.unit.clone(),
        });
        address += if register.is_counter { 4 } else { 2 };
    }
    ModbusMasterSpec {
        link: ModbusLink::Tcp {
            host: host.into(),
            port,
        },
        unit_id,
        poll_ms: 1000,
        timeout_ms: 2000,
        word_order: WordOrder::Big,
        blocks,
    }
}

const DNP3: &str = include_str!("../templates/dnp3.json");

/// One point of a DNP3 plant and how its value moves.
#[derive(Debug, Clone, Deserialize)]
pub struct Dnp3PlantPoint {
    pub key: String,
    pub label: String,
    pub kind: Dnp3PointKind,
    pub index: u16,
    /// Starting value (0 or 1 for binary points).
    pub base: f64,
    #[serde(default)]
    pub variance: f64,
    #[serde(default)]
    pub noise: f64,
    #[serde(default = "default_period")]
    pub period_ms: u64,
    #[serde(default)]
    pub min: Option<f64>,
    #[serde(default)]
    pub max: Option<f64>,
    #[serde(default)]
    pub unit: String,
    /// An input that reports an output: a binary input the state of a
    /// binary output, an analog input an analog output's setpoint.
    #[serde(default)]
    pub follows: Option<String>,
    /// A binary point that must be on for this value to flow: while it is
    /// off, an analog input reads 0 and a counter stops.
    #[serde(default)]
    pub gate: Option<String>,
}

fn default_period() -> u64 {
    60_000
}

/// A plant the DNP3 outstation simulator plays.
#[derive(Debug, Clone, Deserialize)]
pub struct Dnp3Plant {
    pub id: String,
    pub name: String,
    pub description: String,
    pub points: Vec<Dnp3PlantPoint>,
}

static DNP3_PLANTS: LazyLock<Vec<Dnp3Plant>> = LazyLock::new(|| match serde_json::from_str(DNP3) {
    Ok(plants) => plants,
    Err(error) => {
        tracing::error!("built-in DNP3 plants are invalid: {error}");
        Vec::new()
    }
});

/// Every built-in DNP3 plant.
pub fn dnp3() -> &'static [Dnp3Plant] {
    &DNP3_PLANTS
}

pub fn find_dnp3(id: &str) -> Option<&'static Dnp3Plant> {
    dnp3().iter().find(|plant| plant.id == id)
}

/// A master of a simulated DNP3 plant on this computer, its points named
/// and given units the way the plant defines them.
pub fn dnp3_master_for(
    plant: &Dnp3Plant,
    host: &str,
    port: u16,
    outstation_address: u16,
    master_address: u16,
) -> Dnp3MasterSpec {
    Dnp3MasterSpec {
        host: host.into(),
        port,
        master_address,
        outstation_address,
        points: plant
            .points
            .iter()
            .map(|point| Dnp3PointName {
                kind: point.kind,
                index: point.index,
                name: point.label.clone(),
                unit: point.unit.clone(),
            })
            .collect(),
        ..Dnp3MasterSpec::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_builtin_template_loads_and_fits_the_address_space() {
        let templates = modbus();
        assert!(templates.len() >= 6, "templates: {}", templates.len());
        for profile in templates {
            let end = u32::from(profile.base_address) + register_span(profile);
            assert!(end <= 65_536, "{} runs past 65535", profile.id);
        }
    }

    #[test]
    fn master_blocks_cover_the_template_registers_in_order() {
        for profile in modbus() {
            let spec = master_for(profile, "127.0.0.1", 5020, 1);
            assert_eq!(
                spec.point_count(),
                profile.registers.len(),
                "{}",
                profile.id
            );
            let mut expected = u32::from(profile.base_address);
            for block in &spec.blocks {
                assert_eq!(u32::from(block.start), expected, "{}", profile.id);
                expected += u32::from(block.count) * u32::from(block.data_type.words());
            }
            for (block, register) in spec.blocks.iter().zip(&profile.registers) {
                assert_eq!(block.unit, register.unit, "{}", profile.id);
                assert!(!block.name.contains('_'), "{}", block.name);
            }
        }
    }

    #[test]
    fn every_bacnet_template_is_a_device_the_simulator_accepts() {
        let templates = bacnet();
        assert!(templates.len() >= 3, "templates: {}", templates.len());
        for template in templates {
            // The simulator validates the objects (types, binary flags,
            // writable inputs, duplicates) before it binds.
            let device = mt_bacnet::device::start(
                1,
                0,
                0,
                template.points.clone(),
                Box::new(|_| {}),
                Box::new(|_, _| {}),
            )
            .unwrap_or_else(|error| panic!("{}: {error}", template.id));
            device.stop();
            assert!(
                template.points.iter().any(|point| point.writable),
                "{} has nothing to command",
                template.id
            );
        }
    }

    #[test]
    fn every_dnp3_plant_is_one_the_outstation_serves() {
        let plants = dnp3();
        assert!(plants.len() >= 3, "plants: {}", plants.len());
        for plant in plants {
            let mut seen = std::collections::HashSet::new();
            for point in &plant.points {
                // The outstation serves single-bit inputs and outputs, counters and analogs.
                assert!(
                    !matches!(
                        point.kind,
                        Dnp3PointKind::DoubleBitInput | Dnp3PointKind::FrozenCounter
                    ),
                    "{} {}",
                    plant.id,
                    point.key
                );
                assert!(
                    seen.insert((point.kind, point.index)),
                    "{} {}",
                    plant.id,
                    point.key
                );
                let key_of = |key: &str| plant.points.iter().find(|other| other.key == key);
                if let Some(follows) = &point.follows {
                    let source = key_of(follows).expect("follows a point of the plant");
                    assert!(source.kind.writable(), "{} follows an output", point.key);
                    assert_eq!(
                        source.kind.is_binary(),
                        point.kind.is_binary(),
                        "{}",
                        point.key
                    );
                }
                if let Some(gate) = &point.gate {
                    assert!(key_of(gate).expect("gated by a point").kind.is_binary());
                }
            }
            assert!(
                plant
                    .points
                    .iter()
                    .any(|point| point.kind == Dnp3PointKind::BinaryOutput),
                "{} has nothing to command",
                plant.id
            );
            // Controls address one-octet indices.
            assert!(
                plant
                    .points
                    .iter()
                    .all(|point| point.index <= vt_protocol::DNP3_MAX_CONTROL_INDEX)
            );
            let master = dnp3_master_for(plant, "127.0.0.1", 20000, 10, 1);
            assert_eq!(master.points.len(), plant.points.len());
        }
    }
}

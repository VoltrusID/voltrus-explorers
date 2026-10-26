//! Built-in device templates: the Modbus simulator's register maps (the same
//! ones the Modbus Poll app ships, converted from its `templates.ts`) and the
//! BACnet device simulator's plants (from the BACnet Explorer app).

use std::sync::LazyLock;

use mt_bacnet::SimPointDef;

pub use crate::enip::plants::{EnipTemplate, assembly_layout, enip, enip_client_for, find_enip};
use mt_modbus::SimDeviceProfile;
use serde::Deserialize;
use vt_protocol::{
    Area, BacnetClientSpec, DataType, ModbusLink, ModbusMasterSpec, RegisterBlock, WordOrder,
};

const MODBUS: &str = include_str!("../templates/modbus.json");
const BACNET: &str = include_str!("../templates/bacnet.json");

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
}

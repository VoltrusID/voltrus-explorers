//! The Deploy RPC contract: the IDE sends a `bincode`-serialized
//! `CompiledModule` and the server deserializes it with `bincode`. This test
//! pins that wire format so the ESP32 `VLT1` serializer can never silently
//! leak into the gRPC path again.

use mt_plc_types::{CompiledModule, Instruction, Opcode, PlcValue};

fn sample_module() -> CompiledModule {
    CompiledModule {
        name: "Main".to_string(),
        instructions: vec![
            Instruction { opcode: Opcode::PushBool, operand: Some(1) },
            Instruction { opcode: Opcode::PushDint, operand: Some(42) },
        ],
        variable_slots: vec![
            ("motor".to_string(), PlcValue::Bool(false)),
            ("count".to_string(), PlcValue::Dint(0)),
        ],
        pou_table: vec![("Main".to_string(), 0)],
        name_constants: vec!["RUN".to_string()],
        line_map: vec![(1, 0), (2, 2)],
    }
}

#[test]
fn bincode_roundtrip_preserves_module() {
    let module = sample_module();
    let bytes = bincode::serialize(&module).expect("serialize");
    let back: CompiledModule = bincode::deserialize(&bytes).expect("deserialize");

    assert_eq!(back.name, module.name);
    assert_eq!(back.instructions.len(), 2);
    assert_eq!(back.instructions[0].opcode as u8, Opcode::PushBool as u8);
    assert_eq!(back.instructions[1].operand, Some(42));
    assert_eq!(back.variable_slots.len(), 2);
    assert_eq!(back.pou_table.len(), 1);
    assert_eq!(back.name_constants, vec!["RUN".to_string()]);
    assert_eq!(back.line_map.len(), 2);
}

#[test]
fn bincode_payload_is_not_vlt1() {
    // The ESP32 bundle starts with the "VLT1" magic (0x56 0x4C 0x54 0x31).
    // A gRPC deploy payload must never start with it.
    let bytes = bincode::serialize(&sample_module()).expect("serialize");
    assert!(!bytes.starts_with(b"VLT1"));
}

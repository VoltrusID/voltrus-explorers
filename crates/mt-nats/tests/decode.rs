// Smoke test for the new decode + schema (proto) logic added in the
// easy-nats parity build. KV/Object Store/decentralized-auth need a live
// nats-server and are exercised manually against a real server.

use mt_nats::decode::{detect, hex_dump, msgpack_to_json, pretty_json, PayloadFormat};
use mt_nats::schema::SchemaPool;

#[test]
fn detect_json_msgpack_text_binary() {
    assert_eq!(detect(b"{\"a\":1}"), PayloadFormat::Json);
    assert_eq!(detect(b"[1,2,3]"), PayloadFormat::Json);
    // fixmap with 1 entry: key "a"=fixstr, value 1 (fixint)
    assert_eq!(detect(&[0x81, 0xa1, b'a', 0x01]), PayloadFormat::MsgPack);
    assert_eq!(detect(b"hello world"), PayloadFormat::Utf8);
    assert_eq!(detect(&[0x00, 0x01, 0xff]), PayloadFormat::Binary);
    assert_eq!(detect(b""), PayloadFormat::Binary);
}

#[test]
fn pretty_json_roundtrips() {
    let out = pretty_json(b"{\"b\":2,\"a\":1}").unwrap();
    // pretty + sorted-ish (serde_json preserves insertion order of the Value,
    // but pretty-prints with newlines/indents).
    assert!(out.contains("\"a\": 1"));
    assert!(out.contains("\"b\": 2"));
    assert!(out.contains('\n'));
    assert!(pretty_json(b"not json").is_err());
}

#[test]
fn msgpack_decodes_to_json() {
    // {"a": 1} as msgpack
    let mp = [0x81u8, 0xa1, b'a', 0x01];
    let out = msgpack_to_json(&mp).unwrap();
    assert_eq!(out.replace(['\n', ' '], ""), r#"{"a":1}"#);
}

#[test]
fn hex_dump_has_offsets_and_ascii() {
    let out = hex_dump(b"AB");
    assert!(out.starts_with("00000000"));
    assert!(out.contains("41 42"));
    assert!(out.contains("AB"));
}

#[test]
fn schema_proto_parse_bind_decode() {
    let src = r#"
        syntax = "proto3";
        package smoke;
        message Order {
            string id = 1;
            int64 amount = 2;
        }
    "#;
    let mut pool = SchemaPool::new();
    pool.add_proto_source(src).expect("proto compiles");

    let names = pool.message_names();
    assert!(names.iter().any(|n| n == "smoke.Order"), "got {names:?}");

    // Bind a subject pattern to the message.
    pool.bind("orders.*".to_string(), "smoke.Order".to_string());
    assert!(pool.resolve("orders.123").is_some());
    assert!(pool.resolve("shipments.x").is_none());

    // Encode `Order { id: "abc", amount: 7 }` by hand:
    //   field 1 (string), tag 0x0a, len 3, "abc"
    //   field 2 (int64),  tag 0x10, varint 7
    let bytes: Vec<u8> = vec![0x0a, 0x03, b'a', b'b', b'c', 0x10, 0x07];
    let msg = pool.decode_for_subject("orders.123", &bytes).expect("decodes");
    let json = serde_json::to_value(&msg).expect("serializes");
    assert_eq!(json["id"], serde_json::json!("abc"));
    // proto-JSON encodes int64 as a string (precision-loss avoidance).
    assert_eq!(json["amount"], serde_json::json!("7"));
}

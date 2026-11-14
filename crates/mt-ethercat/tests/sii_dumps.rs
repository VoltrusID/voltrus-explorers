//! The SII parser against EEPROM images read from real devices (see `fixtures/README.md`).

use mt_ethercat::sii::{self, Sii, SmType};

fn dump(name: &str) -> Vec<u8> {
    std::fs::read(format!("{}/tests/fixtures/{name}.bin", env!("CARGO_MANIFEST_DIR"))).unwrap()
}

#[test]
fn ek1100() {
    let s = Sii::parse(&dump("ek1100"));
    assert!(s.checksum_ok);
    assert_eq!((s.vendor_id, s.product_code, s.revision), (2, 0x044C_2C52, 0x0012_0000));
    assert_eq!(s.mailbox_protocols, 0);
    assert_eq!(s.size, 2048);
    let g = s.general.unwrap();
    assert_eq!((g.order.as_str(), g.group.as_str()), ("EK1100", "SystemBk"));
    assert_eq!(g.name, "EK1100 EtherCAT-Koppler (2A E-Bus)");
    assert!(s.sync_managers.is_empty());
}

#[test]
fn el2828_outputs() {
    let s = Sii::parse(&dump("el2828"));
    assert!(s.checksum_ok);
    assert_eq!(s.product_code, 0x0B0C_3052);
    assert_eq!(s.sync_managers.len(), 1);
    let sm = s.sync_managers[0];
    assert_eq!((sm.start, sm.len, sm.control, sm.kind), (0x0F00, 1, 0x44, SmType::Outputs));
    assert_eq!(s.rx_pdos.len(), 8);
    assert_eq!(s.rx_pdos[0].name, "Channel 1");
    assert_eq!(s.rx_pdos[0].entries[0].name, "Output");
    assert_eq!(s.rx_pdos.iter().map(|p| p.bits()).sum::<u32>(), 8);
    assert!(s.tx_pdos.is_empty());
    // A vendor category (type 3) sits before the strings; it is listed and skipped.
    assert_eq!(s.categories.first().map(|c| c.0), Some(3));
}

#[test]
fn akd_drive() {
    let s = Sii::parse(&dump("akd"));
    assert!(s.checksum_ok);
    assert_eq!(s.vendor_id, 0x6A);
    assert_eq!(s.std_rx, (0x1800, 1024));
    assert_eq!(s.std_tx, (0x1C00, 1024));
    assert_eq!(sii::mailbox_protocol_names(s.mailbox_protocols), vec!["EoE", "CoE", "FoE"]);
    assert_eq!(s.display_name(), "AKD EtherCAT Drive (CoE)");
    let kinds: Vec<SmType> = s.sync_managers.iter().map(|m| m.kind).collect();
    assert_eq!(kinds, vec![SmType::MailboxOut, SmType::MailboxIn, SmType::Outputs, SmType::Inputs]);
    assert!(!s.tx_pdos.is_empty() && !s.rx_pdos.is_empty());
    let statusword = s.tx_pdos.iter().flat_map(|p| &p.entries).find(|e| e.index == 0x6041).unwrap();
    assert_eq!((statusword.name.as_str(), statusword.bits), ("Statusword", 16));
}

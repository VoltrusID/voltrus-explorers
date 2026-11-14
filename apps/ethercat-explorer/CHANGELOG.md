# EtherCAT Explorer changelog

## 1.0.0

First version.

- Scan an EtherCAT network from a wired Ethernet port (macOS through BPF, Linux through a raw socket): every device
  with its identity, ESC, ports and topology, distributed clock support and propagation delay, AL state and AL status
  code.
- Device views: SII EEPROM contents (general, mailbox, sync managers, strings), ESC registers with decoded meanings,
  sync managers, FMMUs and error counters, the CoE object dictionary with names (SDO Information) and SDO reads,
  and the PDO mapping (from CoE or from the EEPROM).
- AL state changes for all devices or one, stepping through every state on the way up; a device that refuses shows
  its AL status code.
- Cyclic process data with live values per entry, working counter and round-trip statistics; CiA 402 statusword
  decoding.
- Built-in simulator: an EK1100 coupler with EL1008, EL2008, EL3102 and EL4102 terminals wired to each other, and two
  CiA 402 servo drives, answering at register level; pulled-cable and bad-cable faults; frame capture.
- Pro: OP with output writes, SDO writes, register writes and error counter resets, live trends, EEPROM backup,
  pcap export of captured frames, device list CSV.

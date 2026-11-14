# SII EEPROM dumps from real devices

`ek1100.bin` (Beckhoff EK1100), `el2828.bin` (Beckhoff EL2828) and `akd.bin` (Kollmorgen AKD servo drive) are
EEPROM images read from real hardware, taken from the EtherCrab project
(<https://github.com/ethercrab-rs/ethercrab/tree/main/dumps/eeprom>, MIT or Apache-2.0). They check the SII parser
and the header CRC against data no simulator produced.

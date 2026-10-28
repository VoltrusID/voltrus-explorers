# Contributing

Bug reports with the device make/model and a packet capture (Wireshark `.pcapng`) are the most useful thing you can send:
protocol quirks of real devices are what these tools live on.

Pull requests: run `cargo test -p mt-protocol -p mt-modbus -p mt-bacnet -p mt-dnp3 -p mt-s7 -p mt-ble -p mt-enip -p mt-nats -p mt-mqtt -p mt-store -p mt-connect` and `bun run --cwd apps/modbus-poll test`, and
keep each PR to one change.

## Contributor License Agreement

The apps are published under GPL-3.0-or-later and also distributed through Apple's App Stores. The App Store terms
are not compatible with the GPL unless the owner (JoyoDigitama) holds the rights to distribute every line, so
every contributor must sign the [CLA](CLA.md) before a pull request can be merged.

You keep the copyright in your contribution; the CLA gives JoyoDigitama a perpetual, irrevocable license to
distribute it under any terms, including the App Store's. To sign, comment on your pull request:

> I have read the CLA Document and I hereby sign the CLA

A bot records your signature once; it covers all your later contributions. Questions: support@voltrus.id.

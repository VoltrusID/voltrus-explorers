# Contributing

Bug reports with the device make/model and a packet capture (Wireshark `.pcapng`) are the most useful thing you can send:
protocol quirks of real devices are what these tools live on.

Pull requests: run `cargo test -p mt-protocol -p mt-modbus -p mt-bacnet -p mt-dnp3 -p mt-s7 -p mt-ble -p mt-enip -p mt-store` and `bun run --cwd apps/modbus-poll test`, and
keep each PR to one change.

## Contributor terms

The apps are published under GPL-3.0-or-later and also distributed through Apple's App Stores. To keep both possible,
by opening a pull request you agree that:

1. You wrote the contribution, or have the right to submit it.
2. You license it to JoyoDigitama under GPL-3.0-or-later **and** grant JoyoDigitama a perpetual, worldwide,
   non-exclusive, royalty-free right to distribute it under other terms, including the App Store terms.
3. You keep the copyright in your contribution.

Add `Signed-off-by: Your Name <email>` to your commits (`git commit -s`) to confirm.

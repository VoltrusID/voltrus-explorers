# BACnet Explorer on iOS: multicast entitlement

BACnet device discovery is a Who-Is broadcast to UDP 47808 and the I-Am replies. On iOS 14.5+ an app can only send or
receive broadcast/multicast packets with the `com.apple.developer.networking.multicast` entitlement, which Apple grants
per team on request. Connecting to a device by IP address is unicast and needs only the Local Network permission
(`NSLocalNetworkUsageDescription` in `src-tauri/Info.ios.plist`).

## How the app behaves without the entitlement

- The Who-Is send fails; `discover` returns `Who-Is broadcast could not be sent: <os error>` (`crates/mt-bacnet/src/discovery.rs`),
  or finds nothing if the send went through but no reply arrived.
- The discovery sheet shows the error and, on iOS, an explanation: scanning needs Apple's multicast networking permission,
  connect by IP address on the Devices tab, check Settings > Privacy & Security > Local Network.
- Everything else (connect by IP, browse, read/write, COV, trends, alarms, the simulator) works.

## Turning it on

1. Request it at https://developer.apple.com/contact/request/networking-multicast (app, bundle id and how broadcast is
   used: user-initiated Who-Is discovery, no Bonjour alternative).
2. When granted, enable Multicast Networking on the `com.joyodigitama.bacnet-explorer` App ID.
3. Run `release.yml` with `app=bacnet-explorer`, `ios=true`. The job regenerates the provisioning profile, detects the
   entitlement in it and builds with `IOS_MULTICAST=1`, which makes `scripts/build-ios.sh` add
   `src-tauri/entitlements.ios.plist` to the generated entitlements. A profile without the entitlement would make code
   signing fail, which is why the build adds it only when the profile has it.

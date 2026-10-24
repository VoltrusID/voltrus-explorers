# BACnet Explorer on iOS: multicast entitlement

BACnet device discovery is a Who-Is broadcast to UDP 47808 and the I-Am replies. On iOS 14.5+ an app can only send or
receive broadcast/multicast packets with the `com.apple.developer.networking.multicast` entitlement, which Apple grants
per team on request. Connecting to a device by IP address is unicast and needs only the Local Network permission
(`NSLocalNetworkUsageDescription` in `src-tauri/Info.ios.plist`).

## Status

Apple approved the entitlement for team F6Q6UFUK69 and Multicast Networking is enabled on the
`com.joyodigitama.bacnet-explorer` App ID (BACnet Explorer 1.2.2 is the first release built with it). Every iOS build from
`release.yml` regenerates the provisioning profile, requires the entitlement in it and builds with `IOS_MULTICAST=1`,
which makes `scripts/build-ios.sh` add `src-tauri/entitlements.ios.plist` to the generated entitlements. A profile
without the entitlement fails the job (an app without it cannot scan, and signing would fail anyway).

Local builds (`scripts/build-ios.sh` without `IOS_MULTICAST=1` or `--multicast`) omit the entitlement so a development
profile without it still signs; scanning then behaves as below.

## How the app behaves without the entitlement

- The Who-Is send fails; `discover` returns `Who-Is broadcast could not be sent: <os error>` (`crates/mt-bacnet/src/discovery.rs`),
  or finds nothing if the send went through but no reply arrived.
- Everything else (connect by IP, browse, read/write, COV, trends, alarms, the simulator) works.

## How discovery uses it

`discover_devices` binds UDP 47808 (falling back to an ephemeral port when another socket holds it), sends an
Original-Broadcast-NPDU Who-Is to the directed broadcast of every interface plus 255.255.255.255, and for the timeout
collects I-Am replies: broadcast or unicast (Original-Broadcast/Unicast-NPDU) and relayed by a BBMD (Forwarded-NPDU, the
device address is taken from the BVLC header). The COV listener also binds 47808, so scanning while connected with the
listener running does not see broadcast I-Ams (unicast replies still arrive on the source port).

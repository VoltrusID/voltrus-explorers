# Draft reply to App Review: Voltrus Tools 1.0.0, guidelines 4.3 and 4.2.6

Status: sent to App Review.
Do not resubmit the binary; Apple asked for a reply to the thread.

---

Hello,

Thank you for the review. Answers to your questions:

1. What the app does and the problem it solves
Voltrus Tools is a single native workspace for engineers who commission and troubleshoot industrial equipment. In one window it connects to field devices over ten industrial protocols (Modbus TCP/RTU, MQTT with Sparkplug B, BACnet/IP, OPC UA, Siemens S7, EtherNet/IP, DNP3, NATS, serial RS-232/RS-485 and Bluetooth LE; our next update adds MTConnect and FANUC CNC), reads and writes their values, trends them, and shows the decoded traffic. For each protocol it can also run a simulator of the other side, side by side in split panes. The problem it solves: plant and commissioning work on a Mac today means running several Windows-only tools in a virtual machine, one per protocol, with no way to watch a device and its counterpart together.

2. Intended users
Automation, controls and commissioning engineers and system integrators who work with PLCs, meters, drives, building controllers and RTUs from a Mac, and who move between several protocols during one job.

3. Gap it addresses
The established tools for these protocols are Windows-only and single-protocol. On the Mac App Store, our own four apps (Modbus Poll, BACnet Explorer, EtherNet/IP Explorer, DNP3 Explorer) each cover one protocol. Six of the ten protocols in Voltrus Tools (MQTT, OPC UA, S7, NATS, serial and Bluetooth LE) are not available in any app on our account, and the next update adds two more (MTConnect and FANUC CNC). Following earlier App Review feedback under 4.3 on separate single-protocol apps, we consolidated those protocols into this one app instead of publishing more single-purpose apps. Voltrus Tools is also the only one of our apps with multi-protocol workspaces, split panes of a device and its simulator, a command palette and saved multi-device layouts.

4. Beta testing
Testing so far has been internal, through TestFlight builds installed by our own team (five builds between 26 and 28 October); no external testers yet. Each protocol was tested against its built-in simulator and against our own test devices. Internal feedback that changed the build: the workspace layout (how a device pane and its simulator pane split and are restored at launch) was reworked, and each protocol's capabilities were checked against what the app describes, so the submitted build behaves as described.

5. Standalone or suite
It is a standalone product. Our account also has four single-protocol apps for engineers who only need that one protocol. Voltrus Tools differs in scope: ten protocols instead of one, simulators for every protocol, and a multi-device workspace. It is a separate native application (Rust, GPUI), not a wrapper around the single-protocol apps, which are built on a different UI stack (Tauri/WebView).

6. Could it be delivered inside another app
The protocols that exist only in Voltrus Tools cannot be added to the existing single-protocol apps without turning one of them into this suite. We chose one consolidated app over adding more single-protocol apps. We will not submit new standalone protocol apps to the App Store; new protocols go into Voltrus Tools.

7. Shared code with our other apps
Yes: the protocol implementations for Modbus, BACnet, EtherNet/IP and DNP3 are our own Rust libraries shared with our single-protocol apps (and published as open source). The application itself (workspace, sessions engine, simulators for the other protocols, trend renderer, user interface) is specific to Voltrus Tools.

8. Shared code with third-party apps
No. Only standard open-source libraries (for example GPUI, async-opcua, rumqttc), credited in the app.

9. Created for a third party
No. Voltrus Tools is designed, built and published by us (JoyoDigitama, the company behind Voltrus); no template or app generation service was used.

Thank you,
Dzulfikar

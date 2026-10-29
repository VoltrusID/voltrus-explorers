# Draft reply to App Review: Voltrus Tools 1.0.0, guidelines 4.3 and 4.2.6

Status: DRAFT for the owner. Answer 4 needs real facts from the owner before sending.
Do not resubmit the binary; Apple asked for a reply to the thread.

---

Hello,

Thank you for the review. Answers to your questions:

1. What the app does and the problem it solves
Voltrus Tools is a single native workspace for engineers who commission and troubleshoot industrial equipment. In one window it connects to field devices over twelve industrial protocols (Modbus TCP/RTU, MQTT with Sparkplug B, BACnet/IP, OPC UA, Siemens S7, EtherNet/IP, DNP3, NATS, serial RS-232/RS-485, Bluetooth LE, MTConnect and FANUC CNC over FOCAS/Ethernet), reads and writes their values, trends them, and shows the decoded traffic. For each protocol it can also run a simulator of the other side, so an engineer can test a client against a simulated device, or test a device's master against a simulated controller, side by side in split panes. The problem it solves: plant and commissioning work on a Mac today means running several Windows-only tools in a virtual machine, one per protocol, with no way to watch a device and its counterpart together.

2. Intended users
Automation, controls and commissioning engineers, system integrators and OT/IT engineers who work with PLCs, power meters, drives, building controllers, substations RTUs and CNC machines, and who use a Mac as their work computer. Typical users move between protocols during one job (for example a Modbus meter, a BACnet controller and an MQTT gateway on the same site).

3. Gap it addresses
The established tools for these protocols are Windows-only and single-protocol. On the Mac App Store, our own four apps (Modbus Poll, BACnet Explorer, EtherNet/IP Explorer, DNP3 Explorer) each cover one protocol. Eight of the twelve protocols in Voltrus Tools (MQTT, OPC UA, S7, NATS, serial, Bluetooth LE, MTConnect, FANUC) are not available in any app on our account. Following earlier App Review feedback under 4.3 on separate single-protocol apps, we consolidated those protocols into this one app instead of publishing more single-purpose apps. Voltrus Tools is also the only one of our apps with multi-protocol workspaces, split panes of a device and its simulator, a command palette and saved multi-device layouts.

4. Beta testing
[OWNER: describe the TestFlight testing that actually happened. App Store Connect shows an internal "Owner" group with 2 testers and five builds uploaded between 26 and 28 October. Give one or two real examples of feedback and the change made. Do not invent any.]

5. Standalone or suite
It is a standalone product. Our account also has four single-protocol apps for engineers who only need that one protocol. Voltrus Tools differs in scope: twelve protocols instead of one, simulators for every protocol, and a multi-device workspace. It is a separate native application (Rust, GPUI), not a wrapper around the single-protocol apps, which are built on a different UI stack (Tauri/WebView).

6. Could it be delivered inside another app
The eight protocols that exist only in Voltrus Tools cannot be added to the existing single-protocol apps without turning one of them into this suite. We chose one consolidated app over adding more single-protocol apps. We will not submit new standalone protocol apps to the App Store; new protocols go into Voltrus Tools.

7. Shared code with our other apps
Yes: the protocol implementations for Modbus, BACnet, EtherNet/IP and DNP3 are our own Rust libraries shared with our single-protocol apps (and published as open source). The application itself (workspace, sessions engine, simulators for the other protocols, trend renderer, user interface) is specific to Voltrus Tools.

8. Shared code with third-party apps
No third-party app code. It uses standard open-source libraries (for example GPUI for the user interface and protocol libraries such as async-opcua and rumqttc), each credited in the app.

9. Created for a third party
No. Voltrus Tools is designed, built and published by us (JoyoDigitama, the company behind Voltrus); no template or app generation service was used.

Thank you,
Dzulfikar

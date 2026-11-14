import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import { ProProvider } from "@mactools/ui";
import "./styles.css";

const PRO_FEATURES = [
  "OP state, with outputs written from the process data view",
  "SDO writes to the CoE object dictionary",
  "ESC register writes and error counter resets",
  "Live trends of any process data entry",
  "EEPROM backup to a .bin file",
  "Captured frames saved as .pcap for Wireshark",
  "Device list export as CSV",
];

ReactDOM.createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <ProProvider appName="EtherCAT Explorer" features={PRO_FEATURES}>
      <App />
    </ProProvider>
  </React.StrictMode>,
);

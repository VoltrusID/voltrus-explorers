import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import { ProProvider } from "@mactools/ui";
import "./styles.css";

const PRO_FEATURES = [
  "CAN decoder at bit level: stuffing, CRC, ACK and error frames",
  "MCP2515 decoding of SPI transfers",
  "Pattern triggers over several channels",
  "Captures longer than 2 M samples (up to 512 Mi)",
  "Protocol table export to CSV",
  "Export to sigrok .sr and VCD",
];

ReactDOM.createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <ProProvider appName="Voltrus Logic Analyzer" features={PRO_FEATURES}>
      <App />
    </ProProvider>
  </React.StrictMode>,
);

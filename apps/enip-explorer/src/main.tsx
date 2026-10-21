import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import { ProProvider } from "@mactools/ui";
import { EnipDevicePanel } from "./EnipDevicePanel";
import "./styles.css";

// Hash router: a dedicated Tauri window loads `index.html#/sim` to render the
// drive-style device control panel instead of the full app.
const PRO_FEATURES = [
  "Live polling of every tag (free polls 10)",
  "Write tags and CIP attributes, send any CIP service",
  "Live trends for tag values",
  "Implicit I/O (ForwardOpen and cyclic data)",
  "Built-in EtherNet/IP adapter and PLC simulator",
  "Packet log export to CSV",
  "More than one saved tag watch list",
  "Tag watch list export to CSV",
  "Custom response decoders",
];

const route = typeof window !== "undefined" ? window.location.hash : "";

ReactDOM.createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    {route === "#/sim" ? (
      <EnipDevicePanel />
    ) : (
      <ProProvider appName="EtherNet/IP Explorer" features={PRO_FEATURES}>
        <App />
      </ProProvider>
    )}
  </React.StrictMode>,
);

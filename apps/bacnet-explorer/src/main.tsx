import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import { ConnectProvider, ProProvider } from "@mactools/ui";
import { DevicePanel } from "./components/DevicePanel";
import "./styles.css";

// Tiny hash router: a dedicated Tauri window loads `index.html#/sim` to render
// the device control panel instead of the full app.
const PRO_FEATURES = [
  "Write properties with priority and relinquish",
  "Several devices open at once",
  "Export the object list to CSV",
  "Auto-refresh watch list",
  "Live trends with COV subscriptions",
  "Alarm and event summary with acknowledge",
  "Virtual BACnet device simulator",
  "Trend log and schedule viewers",
];

const route = typeof window !== "undefined" ? window.location.hash : "";

ReactDOM.createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    {route === "#/sim" ? (
      <DevicePanel />
    ) : (
      <ProProvider appName="BACnet Explorer" features={PRO_FEATURES}>
        <ConnectProvider>
          <App />
        </ConnectProvider>
      </ProProvider>
    )}
  </React.StrictMode>,
);

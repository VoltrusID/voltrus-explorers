import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import { ProProvider } from "@mactools/ui";
import "./styles.css";

const PRO_FEATURES = [
  "Plots of notify streams with a number format per characteristic",
  "Decode values as integers, floats or text, and show standard GATT names",
  "RSSI chart for nearby devices",
  "Saved write presets and macros",
  "Several notification subscriptions at once",
  "Several connected devices at once",
  "Export the communication log to CSV and copy the whole log",
];

ReactDOM.createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <ProProvider appName="BLE Inspector" features={PRO_FEATURES}>
      <App />
    </ProProvider>
  </React.StrictMode>,
);

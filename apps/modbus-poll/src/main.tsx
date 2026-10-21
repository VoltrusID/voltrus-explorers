import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import ErrorBoundary from "./ErrorBoundary";
import { ProProvider } from "@mactools/ui";
import "./styles.css";

const PRO_FEATURES = [
  "Slave simulator with device templates",
  "Modbus RTU over serial, bus scan and auto-detect (macOS)",
  "Live trends",
  "Network scan",
  "Saved connection profiles",
  "CSV import and export of read blocks",
];

ReactDOM.createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <ErrorBoundary>
      <ProProvider appName="Modbus Poll" features={PRO_FEATURES}>
        <App />
      </ProProvider>
    </ErrorBoundary>
  </React.StrictMode>,
);

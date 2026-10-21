import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import ErrorBoundary from "./ErrorBoundary";
import { ProProvider } from "@mactools/ui";
import "./styles.css";

const PRO_FEATURES = [
  "Write registers and coils",
  "Poll more than 20 values",
  "Slave simulator with device templates",
  "Modbus RTU over serial, bus scan and auto-detect (macOS)",
  "Live trends",
  "Network scan",
  "Several connections open at once, each in its own tab",
  "Log polled values to a CSV file",
  "More than one saved connection profile",
  "Traffic log CSV export and copy",
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

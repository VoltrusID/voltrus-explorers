import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import { ProProvider } from "@mactools/ui";
import "./styles.css";

const PRO_FEATURES = [
  "Outstation simulator with an event buffer",
  "Device templates with live simulated values",
  "Class 1, 2 and 3 event scans",
  "Device controls: time sync, restarts, freeze counters",
  "Live trends",
];

ReactDOM.createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <ProProvider appName="DNP3 Explorer" features={PRO_FEATURES}>
      <App />
    </ProProvider>
  </React.StrictMode>,
);

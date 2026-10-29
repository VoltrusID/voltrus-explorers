import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import { ProProvider } from "@mactools/ui";
import "./styles.css";

const PRO_FEATURES = [
  "Equipment simulator (a wafer processing chamber behind a GEM interface), passive or active",
  "Live trends of status variables",
  "Equipment constant writes with a confirm step and range checks",
  "Report wizard (define, link, enable) with a live S6F11 event table",
  "Remote commands (S2F41) with typed parameters",
  "SML composer for any raw message",
  "CSV export of messages and events",
  "Unlimited saved connection profiles",
];

ReactDOM.createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <ProProvider appName="SECS/GEM Explorer" features={PRO_FEATURES}>
      <App />
    </ProProvider>
  </React.StrictMode>,
);

import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import { ProProvider } from "@mactools/ui";
import "./styles.css";

const PRO_FEATURES = [
  "Unlimited live items (free: 10 MTConnect items stream live)",
  "Live trends for MTConnect samples and FANUC feed, spindle, servo load and positions",
  "MTConnect agent simulator (VMC and lathe) with an SHDR adapter",
  "FANUC CNC simulator (31i-B and 0i-F) with alarms and macro edits",
  "Macro variable writes with a confirm step",
  "Data table CSV export",
  "Unlimited saved connection profiles",
];

ReactDOM.createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <ProProvider appName="CNC Explorer" features={PRO_FEATURES}>
      <App />
    </ProProvider>
  </React.StrictMode>,
);

import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import { ProProvider } from "@mactools/ui";
import "./styles.css";

// The phone layout has no value logging: a log file is appended to for as long as the app runs, which iOS does not allow
// for a file picked in the Files app.
const PHONE = window.innerWidth < 768;

const PRO_FEATURES = [
  "Unlimited live tags (free is limited to 10)",
  "Write values to the PLC",
  "Several PLC connections at once",
  "Live trends",
  "Import and export tag lists as CSV (including TIA Portal tag table exports)",
  ...(PHONE ? [] : ["Log polled values to a CSV file"]),
  "Block manager: list, save and delete blocks",
  "Built-in S7 simulator",
];

ReactDOM.createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <ProProvider appName="S7 Explorer" features={PRO_FEATURES}>
      <App />
    </ProProvider>
  </React.StrictMode>,
);

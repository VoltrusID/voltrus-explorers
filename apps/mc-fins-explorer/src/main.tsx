import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import { ProProvider } from "@mactools/ui";
import "./styles.css";

const PRO_FEATURES = [
  "Unlimited live tags (free is limited to 10)",
  "Write words and bits to the PLC",
  "Several PLC connections at once",
  "Live trends",
  "Import and export tag lists as CSV",
  "Log polled values to a CSV file",
  "Remote RUN and STOP",
  "Built-in MC protocol and FINS PLC simulator",
];

ReactDOM.createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <ProProvider appName="MC & FINS Explorer" features={PRO_FEATURES}>
      <App />
    </ProProvider>
  </React.StrictMode>,
);

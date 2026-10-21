import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import { ProProvider } from "@mactools/ui";
import "./styles.css";

const PRO_FEATURES = [
  "Unlimited live tags (free is limited to 10)",
  "Write values to the PLC",
  "Several PLC connections at once",
  "Live trends",
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

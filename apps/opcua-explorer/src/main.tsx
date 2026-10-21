import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import { ProProvider } from "@mactools/ui";
import "./styles.css";

const PRO_FEATURES = [
  "Unlimited live monitored items and the subscriptions panel",
  "Several server sessions at once",
  "CSV export of monitored values",
  "Live trend charts",
  "Write values to nodes",
  "Certificate management",
  "Device templates for the built-in server",
];

ReactDOM.createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <ProProvider appName="OPC UA Explorer" features={PRO_FEATURES}>
      <App />
    </ProProvider>
  </React.StrictMode>,
);

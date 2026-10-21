import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import { ProProvider } from "@mactools/ui";
import "./styles.css";

const PRO_FEATURES = [
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

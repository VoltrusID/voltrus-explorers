import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import { ProProvider } from "@mactools/ui";
import "./styles.css";

const PRO_FEATURES = [
  "Cyclic transmit of any number of frames",
  "Sending a DBC message by its signal values",
  "Live trends of decoded signals",
  "Recording every frame to a candump file",
  "Trace export as candump or CSV",
  "Replay of candump logs onto the bus, with speed and ID filter",
  "Bus simulator: a genset speaking J1939, CANopen and OBD-II",
  "Unlimited saved interface profiles",
];

ReactDOM.createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <ProProvider appName="CAN Explorer" features={PRO_FEATURES}>
      <App />
    </ProProvider>
  </React.StrictMode>,
);

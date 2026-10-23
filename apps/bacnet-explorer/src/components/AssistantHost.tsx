import { AssistantPanel, type AssistantContext, type AssistantTask } from "@mactools/ui";
import { bacnetConfig, bacnetError } from "../assistantAdapter";

const TASKS: AssistantTask[] = ["ask", "diagnose"];

interface Props {
  device: { port: number; device_id: number } | null;
  openDevices: number;
  error: string | null;
  /** Reads the log when a request is sent. */
  logLines: () => string[];
}

/** The Assistant for BACnet: questions about the session and diagnosis of failed reads or connections. */
export function AssistantHost({ device, openDevices, error, logLines }: Props) {
  const getContext = (): AssistantContext => ({
    config: bacnetConfig({ device, openDevices }),
    log: logLines(),
    error: bacnetError(error),
  });
  return <AssistantPanel protocol="bacnet" tasks={TASKS} getContext={getContext} />;
}

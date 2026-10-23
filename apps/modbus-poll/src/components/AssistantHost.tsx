import { AssistantPanel, commonWordOrder, useProStatus, type AssistantBlock, type AssistantContext, type AssistantTask } from "@mactools/ui";
import { FREE_VALUES } from "../constants";
import { addedMessage, assistantError, sessionConfig, toReadBlocks } from "../assistantAdapter";
import { useStore } from "../store";

const TASKS: AssistantTask[] = ["lookup", "manual", "diagnose", "decode", "ask"];

/** The Assistant for master mode: it sees the connection settings, the blocks and the last log lines, and proposes blocks. */
export function AssistantHost({ error }: { error: string | null }) {
  const { pro, requirePro } = useProStatus();

  const getContext = (): AssistantContext => {
    const s = useStore.getState();
    return {
      config: sessionConfig({
        transport: s.transport,
        tcp: s.tcpFormValues,
        rtu: { baudRate: s.rtuBaud, dataBits: s.rtuDataBits, stopBits: s.rtuStopBits, parity: s.rtuParity, unitId: s.rtuUnitId, timeoutMs: s.rtuTimeout, pollIntervalMs: s.rtuPollInterval },
        endianness: s.endianness,
        oneBasedDisplay: s.useOneBased,
        blocks: s.blocks,
      }),
      block: s.blocks[s.activeBlockTab],
      log: s.log.slice(-40).map((l) => `${l.direction} ${l.message}`),
      error: assistantError(error),
    };
  };

  const onAddBlocks = (proposals: AssistantBlock[]): string => {
    const s = useStore.getState();
    const { blocks, skipped } = toReadBlocks(proposals, s.blocks);
    if (blocks.length === 0) return `Nothing was added. ${skipped.join("; ")}`;
    const next = [...s.blocks, ...blocks];
    const values = next.reduce((n, b) => n + b.quantity, 0);
    if (!pro && values > FREE_VALUES && !requirePro(`More than ${FREE_VALUES} polled values`)) return "Not added: the free version polls up to 20 values.";
    s.setBlocks(next);
    return addedMessage(blocks, skipped, commonWordOrder(proposals), s.endianness);
  };

  return <AssistantPanel protocol="modbus" tasks={TASKS} getContext={getContext} onAddBlocks={onAddBlocks} />;
}

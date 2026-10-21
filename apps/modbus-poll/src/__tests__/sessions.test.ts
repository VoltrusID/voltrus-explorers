import { beforeEach, describe, expect, it } from "vitest";
import { VIRTUAL_PORT } from "../constants";
import { newSessionFields, sessionLabel } from "../sessions";
import { useStore } from "../store";

const reset = () => useStore.setState({ ...newSessionFields(), sessions: [{ id: "c1", fields: null }], activeSessionId: "c1", nextSessionSeq: 2, formKey: 0 });

describe("connection tabs", () => {
  beforeEach(reset);

  it("a new connection starts empty and the first one is parked untouched", () => {
    const s = useStore.getState();
    s.setBlocks([{ name: "a", address: 0, quantity: 1, type: "uint16", area: "holding" }]);
    s.setTcpFormValues({ host: "10.0.0.1", port: 502, unitId: 3, pollInterval: 500 });
    const id = s.addSession();
    const now = useStore.getState();
    expect(id).toBe("c2");
    expect(now.activeSessionId).toBe("c2");
    expect(now.blocks).toEqual([]);
    expect(now.tcpFormValues.host).toBe("127.0.0.1");
    expect(now.sessions.find((x) => x.id === "c1")!.fields!.blocks).toHaveLength(1);
    expect(now.sessions.find((x) => x.id === "c1")!.fields!.tcpFormValues.host).toBe("10.0.0.1");
    expect(now.sessions.find((x) => x.id === "c2")!.fields).toBeNull();
  });

  it("switching swaps every per-connection field and keeps global display options", () => {
    const s = useStore.getState();
    s.setBlocks([{ name: "a", address: 0, quantity: 1, type: "uint16", area: "holding" }]);
    s.setUseOneBased(true);
    s.addSession();
    useStore.getState().setBlocks([{ name: "b", address: 5, quantity: 2, type: "int16", area: "input" }]);
    useStore.getState().addLog("info", "second");
    useStore.getState().switchSession("c1");
    const back = useStore.getState();
    expect(back.blocks.map((b) => b.name)).toEqual(["a"]);
    expect(back.log).toEqual([]);
    expect(back.useOneBased).toBe(true);
    useStore.getState().switchSession("c2");
    expect(useStore.getState().blocks.map((b) => b.name)).toEqual(["b"]);
    expect(useStore.getState().log.map((l) => l.message)).toEqual(["second"]);
  });

  it("switching remounts the connect form", () => {
    const before = useStore.getState().formKey;
    useStore.getState().addSession();
    expect(useStore.getState().formKey).toBe(before + 1);
    useStore.getState().switchSession("c1");
    expect(useStore.getState().formKey).toBe(before + 2);
  });

  it("ids are never reused after a close", () => {
    const s = useStore.getState();
    s.addSession();
    useStore.getState().closeSession("c2");
    expect(useStore.getState().addSession()).toBe("c3");
  });

  it("the last connection can not be closed", () => {
    useStore.getState().closeSession("c1");
    expect(useStore.getState().sessions).toHaveLength(1);
  });

  it("closing the active tab brings its right neighbour forward, else the left one", () => {
    const s = useStore.getState();
    s.addSession();
    useStore.getState().addSession(); // c1 c2 c3, c3 active
    useStore.getState().setBlocks([{ name: "three", address: 0, quantity: 1, type: "uint16", area: "holding" }]);
    useStore.getState().switchSession("c2");
    useStore.getState().closeSession("c2");
    expect(useStore.getState().activeSessionId).toBe("c3");
    expect(useStore.getState().blocks.map((b) => b.name)).toEqual(["three"]);
    useStore.getState().closeSession("c3");
    expect(useStore.getState().activeSessionId).toBe("c1");
  });

  it("closing a background tab leaves the active one alone", () => {
    const s = useStore.getState();
    s.addSession();
    useStore.getState().closeSession("c1");
    expect(useStore.getState().activeSessionId).toBe("c2");
    expect(useStore.getState().sessions.map((x) => x.id)).toEqual(["c2"]);
  });

  it("patches and logs reach a background connection without touching the active one", () => {
    const s = useStore.getState();
    s.addSession(); // c2 active
    useStore.getState().addLogFor("c1", "rx", "from background");
    useStore.getState().addLogFor("c2", "info", "from front");
    expect(useStore.getState().log.map((l) => l.message)).toEqual(["from front"]);
    useStore.getState().switchSession("c1");
    expect(useStore.getState().log.map((l) => l.message)).toEqual(["from background"]);
  });

  it("patching an unknown connection does nothing", () => {
    const before = useStore.getState();
    useStore.getState().patchSession("nope", () => ({ rtuPort: "x" }));
    expect(useStore.getState().sessions).toBe(before.sessions);
  });
});

describe("session labels", () => {
  const base = { transport: "tcp" as const, tcpFormValues: { host: "10.0.0.5", port: 1502 }, rtuPort: "", activeConnProfile: null };

  it("prefers the profile name, then where the connection points", () => {
    expect(sessionLabel({ ...base, activeConnProfile: "Pump house" })).toBe("Pump house");
    expect(sessionLabel(base)).toBe("10.0.0.5:1502");
    expect(sessionLabel({ ...base, transport: "rtu", rtuPort: "/dev/cu.usbserial-0001" })).toBe("cu.usbserial-0001");
    expect(sessionLabel({ ...base, transport: "rtu", rtuPort: VIRTUAL_PORT })).toBe("Virtual RTU");
    expect(sessionLabel({ ...base, transport: "rtu" })).toBe("RTU");
  });
});

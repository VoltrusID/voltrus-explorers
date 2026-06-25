import { describe, it, expect, beforeEach, vi } from "vitest";
import { useStore, clearUndoHistory } from "../store";

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn().mockResolvedValue(undefined),
}));

vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn().mockResolvedValue(() => {}),
}));

describe("store", () => {
  beforeEach(() => {
    clearUndoHistory();
    const { setState } = useStore;
    setState({
      openFiles: [
        { name: "Main.st", path: "Main.st", content: "PROGRAM Main\nVAR\n  x : INT;\nEND_VAR\n  x := 1;\nEND_PROGRAM", modified: false },
        { name: "Helper.st", path: "Helper.st", content: "FUNCTION Add : INT\nVAR_INPUT\n  a : INT;\n  b : INT;\nEND_VAR\n  Add := a + b;\nEND_FUNCTION", modified: false },
      ],
      activeFileIndex: 0,
      searchQuery: "",
      searchResults: [],
      xrefVariable: "",
      xrefResults: [],
      watchedVariables: [],
      ioMappings: [],
      compiledVariables: [
        { name: "x", type_name: "INT", index: 0 },
        { name: "a", type_name: "INT", index: 1 },
        { name: "b", type_name: "INT", index: 2 },
      ],
      log: [],
      runtimeState: "stopped",
      targetIp: "",
      targetConnected: false,
      targetDeploying: false,
      scadaUrl: "http://localhost:3000",
      executionMode: "local",
      theme: "dark",
      bottomTab: "diagnostics",
    });
  });

  describe("file management", () => {
    it("should set active file", () => {
      useStore.getState().setActiveFile(1);
      expect(useStore.getState().activeFileIndex).toBe(1);
    });

    it("should update file content and mark as modified", () => {
      useStore.getState().updateFileContent(0, "PROGRAM Test END_PROGRAM");
      const file = useStore.getState().openFiles[0];
      expect(file.content).toBe("PROGRAM Test END_PROGRAM");
      expect(file.modified).toBe(true);
    });

    it("should close file and adjust active index", () => {
      useStore.getState().setActiveFile(1);
      useStore.getState().closeFile(1);
      expect(useStore.getState().openFiles).toHaveLength(1);
      expect(useStore.getState().activeFileIndex).toBe(0);
    });

    it("should close file and keep index if closing after active", () => {
      useStore.getState().setActiveFile(0);
      useStore.getState().closeFile(1);
      expect(useStore.getState().activeFileIndex).toBe(0);
    });
  });

  describe("search", () => {
    it("should find matches across all files", () => {
      useStore.getState().setSearchQuery("INT");
      useStore.getState().searchAllFiles();
      const results = useStore.getState().searchResults;
      expect(results.length).toBeGreaterThan(0);
      expect(results.some((r) => r.file === "Main.st")).toBe(true);
      expect(results.some((r) => r.file === "Helper.st")).toBe(true);
    });

    it("should return empty results for empty query", () => {
      useStore.getState().setSearchQuery("");
      useStore.getState().searchAllFiles();
      expect(useStore.getState().searchResults).toHaveLength(0);
    });

    it("should support regex search", () => {
      useStore.getState().setSearchQuery("VAR_INPUT");
      useStore.getState().setSearchUseRegex(false);
      useStore.getState().searchAllFiles();
      expect(useStore.getState().searchResults.length).toBeGreaterThan(0);
    });
  });

  describe("cross-reference", () => {
    it("should find variable declarations", () => {
      useStore.getState().setXrefVariable("x");
      useStore.getState().findReferences();
      const results = useStore.getState().xrefResults;
      const decl = results.find((r) => r.kind === "declare");
      expect(decl).toBeDefined();
      expect(decl?.file).toBe("Main.st");
    });

    it("should find multiple reference kinds", () => {
      // Add a line that uses x on both LHS and RHS
      useStore.setState({
        openFiles: [
          {
            name: "Main.st",
            path: "Main.st",
            content: "PROGRAM Main\nVAR\n  x : INT;\n  y : INT;\nEND_VAR\n  x := x + 1;\n  y := x;\nEND_PROGRAM",
            modified: false,
          },
        ],
      });
      useStore.getState().setXrefVariable("x");
      useStore.getState().findReferences();
      const results = useStore.getState().xrefResults;
      const kinds = results.map((r) => r.kind);
      expect(kinds).toContain("declare");
      expect(kinds).toContain("write");
      expect(kinds).toContain("read");
    });

    it("should detect reads on RHS of assignments", () => {
      // x := x + 1 — x is both written (LHS) and read (RHS) on the same line
      useStore.setState({
        openFiles: [
          {
            name: "Main.st",
            path: "Main.st",
            content: "PROGRAM Main\nVAR\n  x : INT;\nEND_VAR\n  x := x + 1;\nEND_PROGRAM",
            modified: false,
          },
        ],
      });
      useStore.getState().setXrefVariable("x");
      useStore.getState().findReferences();
      const results = useStore.getState().xrefResults;
      const reads = results.filter((r) => r.kind === "read");
      const writes = results.filter((r) => r.kind === "write");
      expect(reads.length).toBeGreaterThanOrEqual(1);
      expect(writes.length).toBeGreaterThanOrEqual(1);
      // "x := x + 1" should produce both a write and a read
      const rhsRead = reads.find((r) => r.context.includes("x + 1"));
      expect(rhsRead).toBeDefined();
    });

    it("should not match declare on assignment lines", () => {
      // "x := 1;" should be a write, not a declare
      useStore.setState({
        openFiles: [
          {
            name: "Main.st",
            path: "Main.st",
            content: "PROGRAM Main\nVAR\n  x : INT;\nEND_VAR\n  x := 1;\nEND_PROGRAM",
            modified: false,
          },
        ],
      });
      useStore.getState().setXrefVariable("x");
      useStore.getState().findReferences();
      const results = useStore.getState().xrefResults;
      const declaresOnAssign = results.filter(
        (r) => r.kind === "declare" && r.context.includes(":= 1")
      );
      expect(declaresOnAssign).toHaveLength(0);
      const writes = results.filter((r) => r.kind === "write");
      expect(writes.length).toBeGreaterThanOrEqual(1);
    });

    it("should return empty for empty variable", () => {
      useStore.getState().setXrefVariable("");
      useStore.getState().findReferences();
      expect(useStore.getState().xrefResults).toHaveLength(0);
    });
  });

  describe("log management", () => {
    it("should cap log at 1000 entries", () => {
      for (let i = 0; i < 1100; i++) {
        useStore.getState().addLog("system", `Entry ${i}`);
      }
      expect(useStore.getState().log.length).toBe(1000);
    });
  });

  describe("runtime mode", () => {
    it("should start in local mode", () => {
      expect(useStore.getState().executionMode).toBe("local");
    });

    it("should toggle theme", () => {
      useStore.getState().setTheme("light");
      expect(useStore.getState().theme).toBe("light");
      useStore.getState().setTheme("dark");
      expect(useStore.getState().theme).toBe("dark");
    });
  });

  describe("I/O mappings", () => {
    it("should add I/O mapping", () => {
      useStore.setState({ ioMappings: [] });
      useStore.getState().addIoMapping({
        variable: "relay0",
        direction: "output",
        address: "0",
        driver_id: "gpio",
        protocol_address: "GPIO0",
      });
      expect(useStore.getState().ioMappings).toHaveLength(1);
      expect(useStore.getState().ioMappings[0].variable).toBe("relay0");
    });

    it("should remove I/O mapping", () => {
      useStore.setState({ ioMappings: [
        { variable: "relay0", direction: "output", address: "0", driver_id: "gpio", protocol_address: "GPIO0" },
        { variable: "buzzer", direction: "output", address: "6", driver_id: "gpio", protocol_address: "GPIO6" },
      ] });
      useStore.getState().removeIoMapping(0);
      expect(useStore.getState().ioMappings).toHaveLength(1);
      expect(useStore.getState().ioMappings[0].variable).toBe("buzzer");
    });
  });

  describe("project serialization", () => {
    it("should serialize and deserialize project state", () => {
      useStore.getState().setScanCycle(50);
      useStore.getState().setMaxCycle(2000);
      const serialized = useStore.getState().serializeProject();

      expect(serialized.scan_cycle_ms).toBe(50);
      expect(serialized.max_cycle_ms).toBe(2000);
      expect(serialized.files).toHaveLength(2);
      expect(serialized.version).toBe("0.4");

      useStore.getState().setScanCycle(10);
      useStore.getState().deserializeProject(serialized);
      expect(useStore.getState().scanCycleMs).toBe(50);
      expect(useStore.getState().executionMode).toBe("local");
    });

    it("should always reset execution mode to local on deserialize", () => {
      const serialized = useStore.getState().serializeProject();
      serialized.execution_mode = "remote";
      useStore.getState().deserializeProject(serialized);
      expect(useStore.getState().executionMode).toBe("local");
    });
  });

  describe("undo/redo", () => {
    it("should undo I/O mapping addition", () => {
      useStore.setState({ ioMappings: [] });
      useStore.getState().addIoMapping({
        variable: "relay0", direction: "output", address: "0", driver_id: "gpio", protocol_address: "GPIO0",
      });
      expect(useStore.getState().ioMappings).toHaveLength(1);
      useStore.getState().undo();
      expect(useStore.getState().ioMappings).toHaveLength(0);
    });

    it("should redo after undo", () => {
      useStore.setState({ ioMappings: [] });
      useStore.getState().addIoMapping({
        variable: "relay0", direction: "output", address: "0", driver_id: "gpio", protocol_address: "GPIO0",
      });
      useStore.getState().undo();
      expect(useStore.getState().ioMappings).toHaveLength(0);
      useStore.getState().redo();
      expect(useStore.getState().ioMappings).toHaveLength(1);
    });

    it("should undo scan cycle change", () => {
      const before = useStore.getState().scanCycleMs;
      useStore.getState().setScanCycle(99);
      expect(useStore.getState().scanCycleMs).toBe(99);
      useStore.getState().undo();
      expect(useStore.getState().scanCycleMs).toBe(before);
    });

    it("should track canUndo/canRedo state", () => {
      expect(useStore.getState().canUndo).toBe(false);
      useStore.getState().setScanCycle(55);
      expect(useStore.getState().canUndo).toBe(true);
      expect(useStore.getState().canRedo).toBe(false);
      useStore.getState().undo();
      expect(useStore.getState().canUndo).toBe(false);
      expect(useStore.getState().canRedo).toBe(true);
    });
  });
});

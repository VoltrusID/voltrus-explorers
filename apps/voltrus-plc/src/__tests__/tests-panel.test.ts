import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";
import React, { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import TestsPanel, {
  collectTestFiles,
  extractYaml,
  targetTestPath,
  type TestReportDto,
} from "../components/Tests/TestsPanel";
import { useStore } from "../store";
import type { FileEntry } from "../types";

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

const { invokeMock } = vi.hoisted(() => ({ invokeMock: vi.fn() }));

vi.mock("@tauri-apps/api/core", () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
}));

vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn().mockResolvedValue(() => {}),
}));

const projectFilesFixture: FileEntry[] = [
  { name: "TankLevel.st", path: "/proj/TankLevel.st", is_dir: false },
  { name: "TankLevel.test.yaml", path: "/proj/TankLevel.test.yaml", is_dir: false },
  { name: "sub", path: "/proj/sub", is_dir: true },
];

const mockReport: TestReportDto = {
  passed: false,
  duration_ms: 46,
  file_count: 1,
  scenario_count: 2,
  failure_count: 1,
  error: null,
  suites: [
    {
      path: "TankLevel.test.yaml",
      scenarios: [
        {
          name: "low level opens inlet",
          outcome: "passed",
          duration_ms: 12,
          error: null,
          assertions: [
            { name: "inlet_valve", passed: true, expected: "TRUE", actual: "TRUE" },
          ],
        },
        {
          name: "high level alarms",
          outcome: "failed",
          duration_ms: 34,
          error: null,
          assertions: [
            { name: "high_level_alarm", passed: false, expected: "TRUE", actual: "FALSE (BOOL)" },
          ],
        },
      ],
    },
  ],
};

describe("tests panel helpers", () => {
  it("collectTestFiles keeps only top-level *.test.yaml files", () => {
    const files = collectTestFiles(projectFilesFixture);
    expect(files).toHaveLength(1);
    expect(files[0].name).toBe("TankLevel.test.yaml");
  });

  it("extractYaml pulls the fenced yaml block", () => {
    const response = "Here you go:\n```yaml\nprogram: Tank.st\n```\nDone.";
    expect(extractYaml(response)).toBe("program: Tank.st");
  });

  it("extractYaml falls back to the raw response without fences", () => {
    expect(extractYaml("program: Tank.st")).toBe("program: Tank.st");
  });

  it("targetTestPath derives <Program>.test.yaml next to the project", () => {
    expect(targetTestPath("/proj", "TankLevel.st")).toBe("/proj/TankLevel.test.yaml");
  });
});

describe("TestsPanel", () => {
  let container: HTMLElement;
  let root: Root;
  const renderPanel = () =>
    act(async () => {
      root.render(React.createElement(TestsPanel));
    });

  const click = (el: Element) =>
    act(async () => {
      el.dispatchEvent(new MouseEvent("click", { bubbles: true, cancelable: true }));
    });

  const buttonByText = (text: string) =>
    Array.from(container.querySelectorAll("button")).find(
      (b) => b.textContent === text,
    );

  beforeEach(() => {
    invokeMock.mockReset();
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === "list_project_files") return Promise.resolve(projectFilesFixture);
      if (cmd === "run_project_tests") return Promise.resolve(mockReport);
      return Promise.resolve(undefined);
    });

    useStore.setState({
      projectDir: "/proj",
      openFiles: [
        {
          name: "TankLevel.st",
          path: "/proj/TankLevel.st",
          content: "PROGRAM TankLevel\nVAR\n  level : REAL;\nEND_VAR\nEND_PROGRAM",
          modified: false,
        },
      ],
      activeFileIndex: 0,
      compiledVariables: [
        { name: "level", type_name: "REAL", index: 0 },
        { name: "inlet_valve", type_name: "BOOL", index: 1 },
      ],
      bottomTab: "diagnostics",
    });

    container = document.createElement("div");
    document.body.appendChild(container);
    root = createRoot(container);
  });

  afterEach(() => {
    act(() => {
      root.unmount();
    });
    container.remove();
  });

  it("lists test files before any run", async () => {
    await renderPanel();
    expect(container.textContent).toContain("TankLevel.test.yaml");
    expect(container.textContent).toContain("1 test file found");
  });

  it("renders scenario rows with pass/fail styling from the mock DTO", async () => {
    await renderPanel();
    await click(buttonByText("Run All")!);

    expect(invokeMock).toHaveBeenCalledWith("run_project_tests", {
      projectDir: "/proj",
      junit: null,
    });
    expect(container.textContent).toContain("low level opens inlet");
    expect(container.textContent).toContain("high level alarms");
    expect(container.textContent).toContain("12 ms");
    expect(container.textContent).toContain("1/2 scenarios passed");

    const rows = container.querySelectorAll('[role="button"][aria-expanded]');
    expect(rows).toHaveLength(2);

    const passedIcon = rows[0].firstElementChild as HTMLElement;
    expect(passedIcon.textContent).toBe("\u2713");
    expect(passedIcon.style.color).toBe("rgb(16, 185, 129)");

    const failedIcon = rows[1].firstElementChild as HTMLElement;
    expect(failedIcon.textContent).toBe("\u2717");
    expect(failedIcon.style.color).toBe("var(--danger)");
  });

  it("expands a failed scenario to show expected vs actual", async () => {
    await renderPanel();
    await click(buttonByText("Run All")!);

    const rows = container.querySelectorAll('[role="button"][aria-expanded]');
    expect(rows[1].getAttribute("aria-expanded")).toBe("false");
    expect(container.textContent).not.toContain("FALSE (BOOL)");

    await click(rows[1]);
    expect(rows[1].getAttribute("aria-expanded")).toBe("true");
    expect(container.textContent).toContain("high_level_alarm");
    expect(container.textContent).toContain("expected TRUE");
    expect(container.textContent).toContain("FALSE (BOOL)");
  });

  it("keeps Generate test disabled without an open program", async () => {
    useStore.setState({ openFiles: [], activeFileIndex: 0 });
    await renderPanel();

    const generate = buttonByText("Generate test");
    expect(generate).toBeDefined();
    expect(generate!.disabled).toBe(true);
  });
});

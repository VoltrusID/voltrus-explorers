import { describe, it, expect, beforeEach } from "vitest";
import { useStore, _resetUndoHistory } from "./store";

describe("Store - Widget CRUD", () => {
  beforeEach(() => {
    _resetUndoHistory();
    useStore.setState({
      screens: [
        {
          id: "test",
          name: "Main Screen",
          backgroundColor: "#1e1e2e",
          widgets: [],
        },
      ],
      activeScreenIndex: 0,
      selectedWidgetId: null,
      canUndo: false,
      canRedo: false,
    });
  });

  it("should add a widget", () => {
    useStore.getState().addWidget("gauge", 100, 200);
    const updated = useStore.getState().screens;
    expect(updated[0].widgets).toHaveLength(1);
    expect(updated[0].widgets[0].type).toBe("gauge");
    expect(updated[0].widgets[0].x).toBe(100);
    expect(updated[0].widgets[0].y).toBe(200);
  });

  it("should update a widget", () => {
    useStore.getState().addWidget("gauge", 10, 20);
    const widget = useStore.getState().screens[0].widgets[0];
    useStore.getState().updateWidget(widget.id, { label: "Speed", color: "#ff0000" });
    const updated = useStore.getState().screens[0].widgets[0];
    expect(updated.label).toBe("Speed");
    expect(updated.color).toBe("#ff0000");
  });

  it("should delete a widget", () => {
    useStore.getState().addWidget("button", 50, 50);
    const widget = useStore.getState().screens[0].widgets[0];
    useStore.getState().deleteWidget(widget.id);
    expect(useStore.getState().screens[0].widgets).toHaveLength(0);
  });

  it("should move a widget", () => {
    useStore.getState().addWidget("gauge", 0, 0);
    const widget = useStore.getState().screens[0].widgets[0];
    useStore.getState().moveWidget(widget.id, 100, 200);
    expect(useStore.getState().screens[0].widgets[0].x).toBe(100);
    expect(useStore.getState().screens[0].widgets[0].y).toBe(200);
  });

  it("should resize a widget with min 30 constraint", () => {
    useStore.getState().addWidget("gauge", 0, 0);
    const widget = useStore.getState().screens[0].widgets[0];
    useStore.getState().resizeWidget(widget.id, 10, 10);
    expect(useStore.getState().screens[0].widgets[0].width).toBe(30);
    expect(useStore.getState().screens[0].widgets[0].height).toBe(30);
  });
});

describe("Store - Screen CRUD", () => {
  beforeEach(() => {
    _resetUndoHistory();
    useStore.setState({
      screens: [
        { id: "s1", name: "Main", backgroundColor: "#1e1e2e", widgets: [] },
      ],
      activeScreenIndex: 0,
      selectedWidgetId: null,
      canUndo: false,
      canRedo: false,
    });
  });

  it("should add a screen", () => {
    useStore.getState().addScreen();
    expect(useStore.getState().screens).toHaveLength(2);
    expect(useStore.getState().activeScreenIndex).toBe(1);
  });

  it("should not delete the last screen", () => {
    const id = useStore.getState().screens[0].id;
    useStore.getState().deleteScreen(id);
    expect(useStore.getState().screens).toHaveLength(1);
  });

  it("should update screen properties", () => {
    const id = useStore.getState().screens[0].id;
    useStore.getState().updateScreen(id, {
      name: "Dashboard",
      backgroundColor: "#ffffff",
    });
    expect(useStore.getState().screens[0].name).toBe("Dashboard");
    expect(useStore.getState().screens[0].backgroundColor).toBe("#ffffff");
  });
});

describe("Store - Undo/Redo", () => {
  beforeEach(() => {
    _resetUndoHistory();
    useStore.setState({
      screens: [
        { id: "s1", name: "Main", backgroundColor: "#1e1e2e", widgets: [] },
      ],
      activeScreenIndex: 0,
      selectedWidgetId: null,
      canUndo: false,
      canRedo: false,
    });
  });

  it("should undo widget addition", () => {
    useStore.getState().addWidget("gauge", 10, 10);
    expect(useStore.getState().screens[0].widgets).toHaveLength(1);
    expect(useStore.getState().canUndo).toBe(true);

    useStore.getState().undo();
    expect(useStore.getState().screens[0].widgets).toHaveLength(0);
    expect(useStore.getState().canRedo).toBe(true);
  });

  it("should redo after undo", () => {
    useStore.getState().addWidget("gauge", 10, 10);
    useStore.getState().undo();
    expect(useStore.getState().screens[0].widgets).toHaveLength(0);

    useStore.getState().redo();
    expect(useStore.getState().screens[0].widgets).toHaveLength(1);
  });

  it("should clear redo stack on new action", () => {
    useStore.getState().addWidget("gauge", 10, 10);
    useStore.getState().undo();
    expect(useStore.getState().canRedo).toBe(true);

    useStore.getState().addWidget("button", 20, 20);
    expect(useStore.getState().canRedo).toBe(false);
  });
});

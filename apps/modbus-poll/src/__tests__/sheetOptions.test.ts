import { describe, expect, it } from "vitest";
import { filterSheetOptions, groupSheetOptions, isSearchable, SEARCH_THRESHOLD, type SheetOption } from "../../../../packages/mt-ui/src/sheetOptions";

const opts = (n: number): SheetOption<number>[] => Array.from({ length: n }, (_, i) => ({ value: i, label: `Item ${i}` }));

describe("SheetSelect options", () => {
  it("shows a search field above ten choices unless forced", () => {
    expect(SEARCH_THRESHOLD).toBe(10);
    expect(isSearchable(10, undefined)).toBe(false);
    expect(isSearchable(11, undefined)).toBe(true);
    expect(isSearchable(3, true)).toBe(true);
    expect(isSearchable(40, false)).toBe(false);
  });

  it("filters by label, hint and group, case-insensitively", () => {
    const list: SheetOption[] = [
      { value: "a", label: "Schneider PM5560", hint: "Power meter", group: "Built-in" },
      { value: "b", label: "SMA STP", hint: "Inverter", group: "Built-in" },
      { value: "c", label: "My meter", group: "Custom" },
    ];
    expect(filterSheetOptions(list, "  pm55 ").map((o) => o.value)).toEqual(["a"]);
    expect(filterSheetOptions(list, "inverter").map((o) => o.value)).toEqual(["b"]);
    expect(filterSheetOptions(list, "custom").map((o) => o.value)).toEqual(["c"]);
    expect(filterSheetOptions(list, "").map((o) => o.value)).toEqual(["a", "b", "c"]);
    expect(filterSheetOptions(list, "zzz")).toEqual([]);
    expect(filterSheetOptions(opts(12), "item 1").map((o) => o.value)).toEqual([1, 10, 11]);
  });

  it("groups neighbouring rows and keeps the list order (an ungrouped row after a group stays after it)", () => {
    const list: SheetOption[] = [
      { value: "", label: "Select device…" },
      { value: "a", label: "A", group: "Built-in" },
      { value: "b", label: "B", group: "Built-in" },
      { value: "c", label: "C", group: "Custom" },
      { value: "new", label: "New custom…" },
    ];
    expect(groupSheetOptions(list).map((g) => [g.group, g.items.map((o) => o.value)])).toEqual([
      [undefined, [""]],
      ["Built-in", ["a", "b"]],
      ["Custom", ["c"]],
      [undefined, ["new"]],
    ]);
  });
});

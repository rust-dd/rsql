import { afterEach, describe, expect, it, vi } from "vitest";
import type { Tab, TabType } from "@/types";
import { persistTabs, restoreTabs } from "./tab-persistence";

const tab = (id: string, type: TabType = "query"): Tab => ({
  id,
  type,
  title: id,
  editorValue: `select '${id}'`,
  isExecuting: false,
});

function roundTrip(tabs: Tab[], selectedTabIndex: number) {
  const stored = JSON.parse(JSON.stringify(persistTabs(tabs, selectedTabIndex)));
  const restored = restoreTabs(stored);
  return restored?.tabs[restored.selectedTabIndex]?.id;
}

describe("tab persistence", () => {
  it.each<TabType>([
    "terminal",
    "notify",
  ])("keeps the active tab when a %s tab precedes it", (type) => {
    const tabs = [tab("A"), tab("x", type), tab("B"), tab("C")];
    expect(roundTrip(tabs, 2)).toBe("B");
  });

  it("keeps the active tab when unsaved tabs follow it", () => {
    const tabs = [tab("A"), tab("B"), tab("t", "terminal"), tab("n", "notify")];
    expect(roundTrip(tabs, 1)).toBe("B");
  });

  it("keeps the active tab between several unsaved tabs", () => {
    const tabs = [tab("t", "terminal"), tab("A"), tab("n", "notify"), tab("B"), tab("C")];
    expect(roundTrip(tabs, 4)).toBe("C");
  });

  it("selects the nearest saved tab before an unsaved active tab", () => {
    const tabs = [tab("A"), tab("B"), tab("t", "terminal"), tab("C")];
    expect(roundTrip(tabs, 2)).toBe("B");
  });

  it("selects the first saved tab after an unsaved active tab at the start", () => {
    const tabs = [tab("t", "terminal"), tab("n", "notify"), tab("A"), tab("B")];
    expect(roundTrip(tabs, 0)).toBe("A");
  });

  it("stores no selection when only unsaved tabs are open", () => {
    const stored = persistTabs([tab("t", "terminal")], 0);
    expect(stored).toEqual({ tabs: [], selectedTabIndex: -1, selectedTabId: undefined });
    expect(restoreTabs(stored)).toEqual({ tabs: [], selectedTabIndex: -1 });
  });

  it("drops runtime state from saved tabs", () => {
    const stored = persistTabs([{ ...tab("A"), isExecuting: true, execId: "e1" }], 0);
    expect(stored.tabs[0]).not.toHaveProperty("execId");
    expect(stored.tabs[0].isExecuting).toBe(false);
  });

  it("restores the index stored by older versions that saved no tab id", () => {
    const restored = restoreTabs({ tabs: [tab("A"), tab("B"), tab("C")], selectedTabIndex: 1 });
    expect(restored?.selectedTabIndex).toBe(1);
  });

  it("clamps an out-of-range or malformed stored index", () => {
    const tabs = [tab("A"), tab("B")];
    expect(restoreTabs({ tabs, selectedTabIndex: 7 })?.selectedTabIndex).toBe(1);
    expect(restoreTabs({ tabs, selectedTabIndex: -3 })?.selectedTabIndex).toBe(0);
    expect(restoreTabs({ tabs, selectedTabIndex: "1" })?.selectedTabIndex).toBe(0);
  });

  it("resolves the stored id after malformed tabs are dropped", () => {
    const restored = restoreTabs({
      tabs: [null, { id: "broken" }, tab("A"), tab("B")],
      selectedTabIndex: 3,
      selectedTabId: "B",
    });
    expect(restored?.tabs.map((t) => t.id)).toEqual(["A", "B"]);
    expect(restored?.selectedTabIndex).toBe(1);
  });

  it("falls back to the stored index when the stored id is missing", () => {
    const restored = restoreTabs({
      tabs: [tab("A"), tab("B")],
      selectedTabIndex: 1,
      selectedTabId: "gone",
    });
    expect(restored?.selectedTabIndex).toBe(1);
  });

  it("ignores state that is not a tab list", () => {
    expect(restoreTabs(undefined)).toBeUndefined();
    expect(restoreTabs({ tabs: "nope" })).toBeUndefined();
    expect(restoreTabs({ tabs: [null, 3] })).toEqual({ tabs: [], selectedTabIndex: -1 });
  });

  it.each<TabType>([
    "terminal",
    "notify",
  ])("restores the active query tab after a restart with a %s tab before it", async (type) => {
    const entries = new Map<string, string>();
    vi.stubGlobal("window", {
      localStorage: {
        getItem: (key: string) => entries.get(key) ?? null,
        setItem: (key: string, value: string) => entries.set(key, value),
        removeItem: (key: string) => entries.delete(key),
      },
    });
    const before = await restart();
    before.setState({ tabs: [tab("A"), tab("x", type), tab("B"), tab("C")], selectedTabIndex: 2 });

    const after = (await restart()).getState();
    expect(after.tabs.map((t) => t.id)).toEqual(["A", "B", "C"]);
    expect(after.tabs[after.selectedTabIndex].id).toBe("B");
  });
});

afterEach(() => vi.unstubAllGlobals());

async function restart() {
  vi.resetModules();
  return (await import("./tab-store")).useTabStore;
}

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { useTabStore } from "@/stores/tab-store";
import { ProjectConnectionStatus, type Tab } from "@/types";
import { trackEditorSelection } from "./editor-selection";
import { executeTabQuery } from "./query-execution";

vi.hoisted(() => {
  const entries = new Map<string, string>();
  vi.stubGlobal("localStorage", {
    getItem: (key: string) => entries.get(key) ?? null,
    setItem: (key: string, value: string) => entries.set(key, value),
    removeItem: (key: string) => entries.delete(key),
  });
  vi.stubGlobal("window", { localStorage: globalThis.localStorage });
});

const { driver, history, toastError } = vi.hoisted(() => ({
  driver: {
    executeVirtual: vi.fn(),
    runQuery: vi.fn(),
    closeVirtual: vi.fn(),
    cancelQuery: vi.fn(),
  },
  history: vi.fn(),
  toastError: vi.fn(),
}));
vi.mock("sonner", () => ({ toast: { error: toastError } }));
vi.mock("@/lib/database-driver", () => ({ DriverFactory: { getDriver: () => driver } }));
vi.mock("@/stores/project-store", () => ({
  useProjectStore: {
    getState: () => ({
      projects: { project: { driver: "PGSQL", database: "test" } },
      status: { project: ProjectConnectionStatus.Connected },
    }),
  },
}));
vi.mock("@/stores/history-store", () => ({
  useHistoryStore: { getState: () => ({ addEntry: history }) },
}));
vi.mock("@/stores/schema-index-store", () => ({
  useSchemaIndexStore: { getState: () => ({ invalidateProject: vi.fn() }) },
}));
vi.mock("@/lib/query-helpers", async (original) => ({
  ...(await original<object>()),
  notifyQueryComplete: vi.fn(),
}));

const script = "SELECT 1;\nSELECT 'a;b' AS two;\nDELETE FROM t";
const main = "main" as const;

function tab(): Tab {
  return {
    id: "one",
    type: "query",
    title: "one",
    projectId: "project",
    editorValue: script,
    isExecuting: false,
    isSplit: true,
    splitEditorValue: "SELECT 10;\nSELECT 20",
  };
}
const ranSql = () => driver.executeVirtual.mock.calls.map((call) => call[1]);
const select = (slot: "main" | "split", text: string, start: number, end = start) =>
  trackEditorSelection("one", slot, () => ({ text, start, end }));

beforeEach(() => {
  vi.clearAllMocks();
  driver.executeVirtual.mockResolvedValue(["value", 1, "row", 1, false]);
  driver.closeVirtual.mockResolvedValue(undefined);
  useTabStore.setState({ tabs: [tab()], selectedTabIndex: 0 });
});
afterEach(() => useTabStore.getState().closeAllTabs());

describe("run scope", () => {
  it("runs the statement at the caret and records exactly that SQL", async () => {
    const stop = select(main, script, script.indexOf("'a;b'"));
    await executeTabQuery();
    stop();
    expect(ranSql()).toEqual(["SELECT 'a;b' AS two"]);
    expect(history).toHaveBeenCalledWith(expect.objectContaining({ sql: "SELECT 'a;b' AS two" }));
  });

  it("runs the selection as written, without falling back to the editor", async () => {
    const start = script.indexOf("SELECT 'a");
    const stop = select(main, script, start, script.length);
    await executeTabQuery();
    stop();
    expect(ranSql()).toEqual(["SELECT 'a;b' AS two;\nDELETE FROM t"]);
  });

  it("runs nothing and explains why when the caret is between statements", async () => {
    const sql = "SELECT 1;\n\nSELECT 2;";
    useTabStore.setState({ tabs: [{ ...tab(), editorValue: sql }] });
    const stop = select(main, sql, sql.indexOf("\n\n") + 1);
    await executeTabQuery();
    stop();
    expect(driver.executeVirtual).not.toHaveBeenCalled();
    expect(toastError).toHaveBeenCalledWith(expect.stringMatching(/Run script/));
    expect(useTabStore.getState().tabs[0].isExecuting).toBe(false);
  });

  it("runs the whole editor only through Run script", async () => {
    const stop = select(main, script, 0);
    await executeTabQuery("query", "script");
    stop();
    expect(ranSql()).toEqual([script]);
  });

  it("targets the split editor's own caret and selection", async () => {
    const split = "SELECT 10;\nSELECT 20";
    const stopMain = select(main, script, 0, script.length);
    const stopSplit = select("split", split, split.indexOf("20"));
    await executeTabQuery("split");
    expect(ranSql()).toEqual(["SELECT 20"]);
    expect(useTabStore.getState().tabs[0].splitResult?.rows).toEqual([["row"]]);
    expect(useTabStore.getState().tabs[0].result).toBeUndefined();
    stopMain();
    stopSplit();
  });
});

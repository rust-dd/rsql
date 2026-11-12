import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { useTabStore } from "@/stores/tab-store";
import { ProjectConnectionStatus, type Tab } from "@/types";
import { trackEditorSelection } from "./editor-selection";
import { cancelTabQuery, executeTabQuery } from "./query-execution";

vi.hoisted(() => {
  const entries = new Map<string, string>();
  vi.stubGlobal("localStorage", {
    getItem: (key: string) => entries.get(key) ?? null,
    setItem: (key: string, value: string) => entries.set(key, value),
    removeItem: (key: string) => entries.delete(key),
  });
  vi.stubGlobal("window", { localStorage: globalThis.localStorage });
});

const { driver, projectState, history, toastError } = vi.hoisted(() => ({
  driver: {
    executeVirtual: vi.fn(),
    explainPlan: vi.fn(),
    runQuery: vi.fn(),
    closeVirtual: vi.fn(),
    cancelQuery: vi.fn(),
  },
  projectState: {
    projects: { project: { driver: "PGSQL", database: "test" } },
    status: { project: "Connected" },
    connect: vi.fn(),
  },
  history: vi.fn(),
  toastError: vi.fn(),
}));
vi.mock("sonner", () => ({ toast: { error: toastError } }));
vi.mock("@/lib/database-driver", () => ({ DriverFactory: { getDriver: () => driver } }));
vi.mock("@/stores/project-store", () => ({ useProjectStore: { getState: () => projectState } }));
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

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (error: Error) => void;
  const promise = new Promise<T>((done, fail) => {
    resolve = done;
    reject = fail;
  });
  return { promise, resolve, reject };
}
function tab(id = "one"): Tab {
  return {
    id,
    type: "query",
    title: id,
    projectId: "project",
    editorValue: "SELECT 1",
    isExecuting: false,
    queryTimeout: 5000,
  };
}
const active = () => useTabStore.getState().tabs[0];

beforeEach(() => {
  vi.clearAllMocks();
  driver.executeVirtual.mockReset();
  driver.runQuery.mockReset();
  driver.closeVirtual.mockResolvedValue(undefined);
  driver.cancelQuery.mockResolvedValue(true);
  projectState.status.project = ProjectConnectionStatus.Connected;
  useTabStore.setState({ tabs: [tab()], selectedTabIndex: 0 });
});
afterEach(() => useTabStore.getState().closeAllTabs());

describe("query ownership", () => {
  it("ignores an old result and does not clear a newer spinner", async () => {
    const old = deferred<unknown>();
    const next = deferred<unknown>();
    driver.executeVirtual.mockReturnValueOnce(old.promise).mockReturnValueOnce(next.promise);
    const first = executeTabQuery();
    const oldExec = active().execId;
    const second = executeTabQuery();
    const newExec = active().execId;
    old.resolve(["value", 5000, "old", 1, false]);
    await first;
    expect(active().execId).toBe(newExec);
    expect(active().isExecuting).toBe(true);
    expect(active().result).toBeUndefined();
    expect(driver.cancelQuery).toHaveBeenCalledWith(oldExec);
    expect(driver.closeVirtual).toHaveBeenCalledWith(
      "project",
      driver.executeVirtual.mock.calls[0][2],
    );
    next.resolve(["value", 1, "new", 2, false]);
    await second;
    expect(active().result?.rows).toEqual([["new"]]);
    expect(history).toHaveBeenCalledTimes(1);
  });

  it.each([
    "closeTab",
    "closeAllTabs",
    "closeOtherTabs",
  ] as const)("%s cancels a pending query and releases late results", async (action) => {
    const gate = deferred<unknown>();
    driver.executeVirtual.mockReturnValueOnce(gate.promise);
    const task = executeTabQuery();
    const exec = active().execId;
    useTabStore.getState().openTab("project");
    if (action === "closeOtherTabs") useTabStore.getState().closeOtherTabs(1);
    else if (action === "closeTab") useTabStore.getState().closeTab(0);
    else useTabStore.getState().closeAllTabs();
    gate.resolve(["value", 5000, "old", 1, false]);
    await task;
    expect(driver.cancelQuery).toHaveBeenCalledWith(exec);
    expect(driver.closeVirtual).toHaveBeenCalledWith(
      "project",
      driver.executeVirtual.mock.calls[0][2],
    );
    expect(history).not.toHaveBeenCalled();
  });

  it("cannot start SQL when cancelled while connecting", async () => {
    const gate = deferred<void>();
    projectState.status.project = ProjectConnectionStatus.Disconnected;
    projectState.connect.mockImplementationOnce(async () => {
      await gate.promise;
      projectState.status.project = ProjectConnectionStatus.Connected;
    });
    const task = executeTabQuery();
    cancelTabQuery();
    gate.resolve();
    await task;
    expect(driver.executeVirtual).not.toHaveBeenCalled();
    expect(active().result?.status).toBe("cancelled");
  });

  it("does not write an old error into a newer execution", async () => {
    const gate = deferred<unknown>();
    driver.executeVirtual
      .mockReturnValueOnce(gate.promise)
      .mockResolvedValueOnce(["value", 1, "new", 1, false]);
    const first = executeTabQuery();
    await executeTabQuery();
    gate.reject(new Error("old error"));
    await first;
    expect(active().result?.rows).toEqual([["new"]]);
  });

  it("releases a retained result when its tab closes", async () => {
    driver.executeVirtual.mockResolvedValueOnce(["value", 5000, "first", 1, false]);
    await executeTabQuery();
    const id = active().virtualQuery?.queryId;
    useTabStore.getState().closeTab(0);
    expect(driver.closeVirtual).toHaveBeenCalledWith("project", id);
  });

  it("passes timeout and execution ownership to EXPLAIN", async () => {
    driver.explainPlan.mockResolvedValueOnce([{ Plan: { "Node Type": "Result" } }]);
    await executeTabQuery("explain");
    expect(driver.explainPlan).toHaveBeenCalledWith(
      "project",
      "SELECT 1",
      false,
      5000,
      expect.any(String),
    );
    expect(driver.runQuery).not.toHaveBeenCalled();
    expect(active().explainResult?.Plan["Node Type"]).toBe("Result");
    expect(active().isExecuting).toBe(false);
  });

  it("split cancellation leaves the main execution running", async () => {
    const main = deferred<unknown>();
    const split = deferred<unknown>();
    driver.executeVirtual.mockReturnValueOnce(main.promise).mockReturnValueOnce(split.promise);
    useTabStore.setState({ tabs: [{ ...tab(), isSplit: true, splitEditorValue: "SELECT 2" }] });
    const first = executeTabQuery();
    const second = executeTabQuery("split");
    const splitId = active().splitExecId;
    cancelTabQuery(true);
    expect(driver.cancelQuery).toHaveBeenCalledWith(splitId);
    expect(active().isExecuting).toBe(true);
    expect(driver.executeVirtual).toHaveBeenLastCalledWith(
      "project",
      "SELECT 2",
      expect.any(String),
      expect.any(Number),
      5000,
      splitId,
    );
    split.resolve(["value", 1, "old", 1, false]);
    main.resolve(["value", 1, "main", 1, false]);
    await Promise.all([first, second]);
    expect(active().splitResult?.status).toBe("cancelled");
    expect(active().result?.rows).toEqual([["main"]]);
    expect(driver.runQuery).not.toHaveBeenCalled();
  });

  it("each pane pages its own large result and releases only its own", async () => {
    driver.executeVirtual.mockResolvedValue(["value", 5000, "first", 1, true]);
    useTabStore.setState({ tabs: [{ ...tab(), isSplit: true, splitEditorValue: "SELECT 2" }] });
    await executeTabQuery();
    await executeTabQuery("split");
    const mainId = active().virtualQuery?.queryId;
    const splitId = active().splitVirtualQuery?.queryId;
    expect(splitId).toBe(driver.executeVirtual.mock.calls[1][2]);
    expect(splitId).not.toBe(mainId);
    expect(active().splitVirtualQuery?.totalRows).toBe(5000);
    expect(active().splitResult?.capped).toBe(true);
    expect(driver.closeVirtual).not.toHaveBeenCalled();

    await executeTabQuery("split");
    expect(driver.closeVirtual).toHaveBeenCalledExactlyOnceWith("project", splitId);
    const rerunId = active().splitVirtualQuery?.queryId;
    useTabStore.getState().toggleSplit(active().id);
    expect(driver.closeVirtual).toHaveBeenLastCalledWith("project", rerunId);
    expect(active().splitVirtualQuery).toBeUndefined();
    expect(active().virtualQuery?.queryId).toBe(mainId);
    expect(driver.closeVirtual).not.toHaveBeenCalledWith("project", mainId);

    await executeTabQuery();
    expect(driver.closeVirtual).toHaveBeenLastCalledWith("project", mainId);
  });
});

describe("explain targeting", () => {
  const plan = [{ Plan: { "Node Type": "ModifyTable" } }];
  const script = "SELECT 1;\nDELETE FROM t;\nUPDATE t SET a = ';'";
  let untrack: (() => void) | undefined;
  const editor = (start: number, end = start) => {
    untrack = trackEditorSelection("one", "main", () => ({ text: script, start, end }));
  };
  beforeEach(() => {
    useTabStore.setState({ tabs: [{ ...tab(), editorValue: script }], selectedTabIndex: 0 });
  });
  afterEach(() => untrack?.());

  it("explains only the statement at the caret, without ANALYZE", async () => {
    driver.explainPlan.mockResolvedValueOnce(plan);
    editor(script.indexOf("DELETE") + 2);
    await executeTabQuery("explain");
    expect(driver.explainPlan).toHaveBeenCalledWith(
      "project",
      "DELETE FROM t",
      false,
      5000,
      expect.any(String),
    );
  });

  it("analyzes the selected statement only when asked to", async () => {
    driver.explainPlan.mockResolvedValueOnce(plan);
    const start = script.indexOf("UPDATE");
    editor(start, script.length);
    await executeTabQuery("analyze");
    expect(driver.explainPlan).toHaveBeenCalledWith(
      "project",
      "UPDATE t SET a = ';'",
      true,
      5000,
      expect.any(String),
    );
  });

  it.each([
    ["a selection spanning statements", () => editor(0, script.length)],
    ["several statements and no editor caret", () => {}],
  ])("runs nothing for %s", async (_, setup) => {
    setup();
    useTabStore.setState({ tabs: [{ ...tab(), editorValue: script, result: undefined }] });
    await executeTabQuery("explain");
    expect(driver.explainPlan).not.toHaveBeenCalled();
    expect(driver.runQuery).not.toHaveBeenCalled();
    expect(toastError).toHaveBeenCalledWith(expect.stringMatching(/statements/));
    expect(active().isExecuting).toBe(false);
    expect(history).not.toHaveBeenCalled();
  });
});

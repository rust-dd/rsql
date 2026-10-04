import { mockIPC } from "@tauri-apps/api/mocks";
import { StrictMode, useState } from "react";
import { createRoot } from "react-dom/client";
import { Toaster } from "sonner";
import { ResultsPanel } from "../../src/components/results-panel";
import { TabBar } from "../../src/components/tab-bar";
import * as cache from "../../src/lib/virtual-cache";
import { encodeRows } from "../../src/lib/wire";
import { useProjectStore } from "../../src/stores/project-store";
import { useTabStore } from "../../src/stores/tab-store";
import { useUIStore } from "../../src/stores/ui-store";
import type { QueryResult, Tab } from "../../src/types";
import "../../src/index.css";

let failPages = false;
mockIPC(
  async (command, args) => {
    if (command === "pgsql_fetch_page") {
      await new Promise((resolve) => setTimeout(resolve, 100));
      if (failPages) throw new Error("The result is unavailable. Run the query again.");
      return encodeRows(
        Array.from({ length: Number(args?.limit) }, (_, i) => [
          String(Number(args?.offset) + i),
          i % 2 ? "" : null,
        ]),
      );
    }
    if (command === "pgsql_execute_virtual") return ["value", 0, "", 1, false];
    return [];
  },
  { shouldMockEvents: true },
);

const sample: QueryResult = {
  columns: ["id", "value", "value", "description"],
  rows: [
    ["1", null, "", "árvíztűrő 🦀\nsecond line"],
    ["2", "null", "normal", "Long content ".repeat(60)],
  ],
  time: 12,
};
const tabs: Tab[] = ["Customers", "Orders"].map((title, i) => ({
  id: `ui-fixture-${i}`,
  type: "query",
  title,
  projectId: "fixture",
  editorValue: "SELECT 1",
  isExecuting: false,
  result: i ? { columns: ["order"], rows: [["Order 1"]], time: 4 } : sample,
}));
useTabStore.setState({ tabs, selectedTabIndex: 0 });
useProjectStore.setState({
  projects: {
    fixture: {
      driver: "PGSQL",
      database: "UI fixtures",
      username: "",
      password: "",
      host: "",
      port: "",
      ssl: "false",
      sshEnabled: "false",
      sshHost: "",
      sshPort: "",
      sshUser: "",
      sshPassword: "",
      sshKeyPath: "",
    },
  },
});

function setResult(result: QueryResult, virtual = false) {
  const store = useTabStore.getState();
  const tab = store.tabs[store.selectedTabIndex];
  if (!tab) return;
  if (tab.virtualQuery) cache.clearQuery(tab.virtualQuery.queryId);
  const queryId = crypto.randomUUID();
  store.setVirtualQuery(
    tab.id,
    virtual
      ? {
          queryId,
          columns: result.columns,
          totalRows: 10000,
          pageSize: 100,
          colCount: result.columns.length,
          time: result.time,
        }
      : undefined,
  );
  if (virtual) cache.setPage(queryId, 0, result.rows);
  store.updateResult(tab.id, result);
}

function FixtureApp() {
  const [compact, setCompact] = useState(false);
  return (
    <div className="flex h-screen flex-col gap-3 bg-background p-4 text-foreground">
      <div className="flex flex-wrap items-center gap-2 text-xs">
        <strong>Result UI test fixtures</strong>
        <button type="button" onClick={() => setResult(sample)}>
          Sample rows
        </button>
        <button type="button" onClick={() => setResult({ columns: ["value"], rows: [], time: 2 })}>
          Empty SELECT
        </button>
        <button
          type="button"
          onClick={() =>
            setResult({
              columns: [],
              rows: [],
              time: 4,
              status: "error",
              message: 'relation "missing_table" does not exist',
            })
          }
        >
          Query error
        </button>
        <button
          type="button"
          onClick={() =>
            setResult(
              {
                columns: ["id", "value"],
                rows: Array.from({ length: 100 }, (_, i) => [String(i), i % 2 ? "" : null]),
                time: 32,
              },
              true,
            )
          }
        >
          Large result
        </button>
        <label>
          <input
            type="checkbox"
            onChange={(event) => {
              failPages = event.target.checked;
            }}
          />
          Fail page requests
        </label>
        <button type="button" onClick={() => setCompact(!compact)}>
          Toggle compact width
        </button>
        <button type="button" onClick={() => useUIStore.getState().toggleTheme()}>
          Toggle theme
        </button>
      </div>
      <div
        className="flex min-h-0 flex-1 flex-col self-center overflow-hidden rounded border border-border"
        style={{ width: compact ? 720 : "100%", maxWidth: "100%" }}
      >
        <TabBar />
        <div className="min-h-0 flex-1">
          <ResultsPanel />
        </div>
      </div>
      <Toaster />
    </div>
  );
}
const root = document.getElementById("root");
if (root)
  createRoot(root).render(
    <StrictMode>
      <FixtureApp />
    </StrictMode>,
  );

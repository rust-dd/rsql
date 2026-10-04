import * as monaco from "monaco-editor";
import { useEffect, useState } from "react";
import { createRoot } from "react-dom/client";
import { QueryEditor } from "../../src/components/query-editor";
import { ResultsGrid } from "../../src/components/results-grid";
import { useTabStore } from "../../src/stores/tab-store";
import "../../src/monaco/setup";
import "../../src/index.css";

useTabStore.setState({
  selectedTabIndex: 0,
  tabs: [{ id: "editor-fixture", type: "query", title: "Editor test", isExecuting: false }],
});

function EditorFixture() {
  const [sql, setSql] = useState("SELECT 1 AS value;\n");
  const [edits, setEdits] = useState(new Map<string, string>());
  const [executions, setExecutions] = useState(0);
  const [diagnostics, setDiagnostics] = useState<string[]>([]);
  useEffect(() => {
    const subscription = monaco.editor.onDidChangeMarkers(() => {
      setDiagnostics(monaco.editor.getModelMarkers({}).map((marker) => marker.message));
    });
    return () => subscription.dispose();
  }, []);
  return (
    <main className="flex h-screen flex-col bg-background text-foreground">
      <header className="p-3">Production SQL editor and editable grid compatibility</header>
      <div className="flex gap-3 px-3">
        <input
          aria-label="SQL fixture"
          value={sql}
          onChange={(event) => setSql(event.target.value)}
          className="w-96 border p-2"
        />
        <button
          type="button"
          onClick={() => {
            const editor = monaco.editor.getEditors()[0];
            editor?.focus();
            editor?.setPosition({ lineNumber: 1, column: sql.length + 1 });
            editor?.trigger("fixture", "editor.action.triggerSuggest", {});
          }}
        >
          Show completions
        </button>
      </div>
      <QueryEditor value={sql} onChange={setSql} onExecute={() => setExecutions((n) => n + 1)} />
      <output className="p-3" aria-label="Executed queries">
        Executed: {executions}
      </output>
      <output className="px-3" aria-label="SQL diagnostics">
        {JSON.stringify(diagnostics)}
      </output>
      <div className="h-64">
        <ResultsGrid
          columns={["id", "note"]}
          rows={[
            ["1", "Editable text"],
            ["2", "Second row"],
          ]}
          isEditing
          cellEdits={edits}
          onCellEdit={(row, column, value) => {
            setEdits((previous) => new Map(previous).set(`${row}:${column}`, value));
          }}
        />
      </div>
      <output className="p-3" aria-label="Saved cell edits">
        {JSON.stringify([...edits])}
      </output>
    </main>
  );
}

const root = document.getElementById("root");
if (root) createRoot(root).render(<EditorFixture />);

import * as monaco from "monaco-editor";
import { useState } from "react";
import { flushSync } from "react-dom";
import { createRoot } from "react-dom/client";
import { QueryEditor } from "../../src/components/query-editor";
import { useActiveTab, useTabStore } from "../../src/stores/tab-store";
import "../../src/monaco/setup";
import "../../src/index.css";

useTabStore.setState({ tabs: [], selectedTabIndex: -1 });

const store = () => useTabStore.getState();
const update = (action: () => void) => flushSync(action);
const waitFor = async (condition: () => boolean) => {
  const deadline = performance.now() + 10_000;
  while (!condition()) {
    if (performance.now() > deadline) throw new Error("Timed out waiting for editor");
    await new Promise((resolve) => setTimeout(resolve, 20));
  }
};
const editor = (index = 0) => monaco.editor.getEditors()[index];
const assert = (condition: boolean, message: string) => {
  if (!condition) throw new Error(message);
};
const expectText = (text: string, index = 0) =>
  assert(
    editor(index)?.getValue() === text,
    `Expected ${JSON.stringify(text)} in pane ${index}, got ${JSON.stringify(editor(index)?.getValue())}`,
  );
const type = (text: string, index = 0) => {
  const target = editor(index);
  target.pushUndoStop();
  update(() => target.trigger("test", "type", { text }));
  target.pushUndoStop();
};
const undo = async (index = 0) => {
  await flushSync(() => editor(index).getModel()?.undo());
};
const redo = async (index = 0) => {
  await flushSync(() => editor(index).getModel()?.redo());
};

function Fixture() {
  const tab = useActiveTab();
  const [result, setResult] = useState("Ready");

  async function run() {
    setResult("Running");
    try {
      update(() => store().closeAllTabs());
      for (const firstText of ["", "SELECT 1;"]) {
        update(() => store().openTab(undefined, firstText));
        await waitFor(() => monaco.editor.getEditors().length === 1);
        const firstId = store().tabs[0].id;
        update(() => store().openTab());
        type("SELECT 2;");
        update(() => store().selectTab(0));
        await undo();
        expectText(firstText);
        update(() => store().selectTab(1));
        await undo();
        expectText("");
        update(() => store().selectTab(0));
        type(" -- first");
        await undo();
        expectText(firstText);
        update(() => store().selectTab(1));
        await redo();
        expectText("SELECT 2;");
        update(() => store().selectTab(0));
        await redo();
        assert(editor().getValue().includes("-- first"), "First tab lost its redo history");

        const firstModel = editor().getModel();
        update(() => store().setProjectId(firstId, "other-connection"));
        assert(!firstModel?.isDisposed(), "Connection changes disposed the SQL model");
        update(() => store().toggleSplit(firstId));
        await waitFor(() => monaco.editor.getEditors().length === 2);
        type("SELECT 'right';", 1);
        const splitModel = editor(1).getModel();
        update(() => store().toggleSplit(firstId));
        await waitFor(() => monaco.editor.getEditors().length === 1);
        await undo();
        expectText(firstText);
        update(() => store().toggleSplit(firstId));
        await waitFor(() => monaco.editor.getEditors().length === 2);
        expectText("SELECT 'right';", 1);
        await undo(1);
        expectText("", 1);
        await redo(1);
        expectText("SELECT 'right';", 1);

        update(() => store().openMonitorTab("other-connection"));
        assert(monaco.editor.getEditors().length === 0, "Non-query tab retained editor views");
        update(() => store().selectTab(0));
        await waitFor(() => monaco.editor.getEditors().length === 2);
        await redo();
        assert(editor().getValue().includes("-- first"), "Unmounting lost the main redo history");
        await undo(1);
        expectText("", 1);

        update(() => store().closeTab(0));
        assert(firstModel?.isDisposed() === true, "Closing a tab leaked its main model");
        assert(splitModel?.isDisposed() === true, "Closing a tab leaked its split model");
        update(() => store().closeAllTabs());
      }

      for (const close of [() => store().closeAllTabs(), () => store().closeOtherTabs(1)]) {
        update(() => store().openTab());
        await waitFor(() => monaco.editor.getEditors().length === 1);
        const closingModel = editor().getModel();
        update(() => store().openTab());
        const keptModel = editor().getModel();
        update(close);
        assert(closingModel?.isDisposed() === true, "Bulk close leaked an inactive model");
        if (store().tabs.length) {
          assert(!keptModel?.isDisposed(), "Close others disposed the retained model");
        } else {
          assert(keptModel?.isDisposed() === true, "Close all leaked the active model");
        }
        update(() => store().closeAllTabs());
      }
      assert(monaco.editor.getModels().length === 0, "Closed tabs left SQL models behind");
      setResult(
        "PASS: independent undo/redo, split panes, unmounts, connection changes, tab cleanup",
      );
    } catch (error) {
      setResult(`FAIL: ${error instanceof Error ? error.message : String(error)}`);
    }
  }

  const main = tab?.type === "query" && (
    <QueryEditor
      tabId={tab.id}
      value={tab.editorValue ?? ""}
      onChange={(value) => store().updateContent(tab.id, value)}
      onExecute={() => {}}
    />
  );

  return (
    <main className="flex h-screen flex-col">
      <button type="button" onClick={() => void run()} disabled={result === "Running"}>
        Run undo history regression
      </button>
      <output>{result}</output>
      {tab?.isSplit ? (
        <div className="flex flex-1">
          <div className="flex flex-1 flex-col">{main}</div>
          <QueryEditor
            tabId={tab.id}
            slot="split"
            value={tab.splitEditorValue ?? ""}
            onChange={(value) => store().updateSplitContent(tab.id, value)}
            onExecute={() => {}}
          />
        </div>
      ) : (
        main
      )}
    </main>
  );
}

const root = document.getElementById("root");
if (root) createRoot(root).render(<Fixture />);

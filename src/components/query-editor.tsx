import Editor, { useMonaco } from "@monaco-editor/react";
import type * as Monaco from "monaco-editor";
import { useCallback, useEffect, useRef, useState } from "react";
import { trackEditorSelection } from "@/lib/editor-selection";
import { trackTabEditor } from "@/lib/tab-resources";
import { useEditorScopeStore } from "@/stores/editor-scope-store";
import { useUIStore } from "@/stores/ui-store";

interface QueryEditorProps {
  tabId: string;
  slot?: "main" | "split";
  value: string;
  onChange: (value: string) => void;
  onExecute: () => void;
  onExecuteScript?: () => void;
  onExplain?: () => void;
  onAnalyze?: () => void;
}

export function QueryEditor({
  tabId,
  slot = "main",
  value,
  onChange,
  onExecute,
  onExecuteScript,
  onExplain,
  onAnalyze,
}: QueryEditorProps) {
  const monaco = useMonaco();
  const modelPath = `rsql://query/${encodeURIComponent(tabId)}/${slot}.sql`;
  const theme = useUIStore((s) => s.theme);
  const [editor, setEditor] = useState<Monaco.editor.IStandaloneCodeEditor>();
  const onExecuteRef = useRef(onExecute);
  onExecuteRef.current = onExecute;
  const onExecuteScriptRef = useRef(onExecuteScript);
  onExecuteScriptRef.current = onExecuteScript;
  const onExplainRef = useRef(onExplain);
  onExplainRef.current = onExplain;
  const onAnalyzeRef = useRef(onAnalyze);
  onAnalyzeRef.current = onAnalyze;

  useEffect(() => {
    if (!monaco) return;
    trackTabEditor(tabId, slot, () =>
      monaco.editor.getModel(monaco.Uri.parse(modelPath))?.dispose(),
    );
  }, [monaco, tabId, slot, modelPath]);

  useEffect(() => {
    if (!editor || !monaco) return;
    const uri = monaco.Uri.parse(modelPath).toString();
    return trackEditorSelection(tabId, slot, () => {
      const model = editor.getModel();
      const selection = editor.getSelection();
      if (!model || !selection || model.uri.toString() !== uri) return undefined;
      return {
        text: model.getValue(),
        start: model.getOffsetAt(selection.getStartPosition()),
        end: model.getOffsetAt(selection.getEndPosition()),
      };
    });
  }, [editor, monaco, tabId, slot, modelPath]);

  useEffect(() => {
    if (!editor) return;
    const { setScope } = useEditorScopeStore.getState();
    const report = () =>
      setScope(tabId, slot, editor.getSelection()?.isEmpty() === false ? "selection" : "statement");
    report();
    const subscription = editor.onDidChangeCursorSelection(report);
    return () => {
      subscription.dispose();
      setScope(tabId, slot, undefined);
    };
  }, [editor, tabId, slot]);

  const handleMount = useCallback(
    (editor: Monaco.editor.IStandaloneCodeEditor, monaco: typeof Monaco) => {
      setEditor(editor);
      editor.addAction({
        id: "run-query",
        label: "Run Selection or Statement",
        keybindings: [monaco.KeyMod.CtrlCmd | monaco.KeyCode.Enter],
        run: () => onExecuteRef.current(),
      });
      editor.addAction({
        id: "run-script",
        label: "Run Script",
        keybindings: [monaco.KeyMod.CtrlCmd | monaco.KeyMod.Alt | monaco.KeyCode.Enter],
        run: () => onExecuteScriptRef.current?.(),
      });
      editor.addAction({
        id: "explain-query",
        label: "Explain Statement (estimated plan)",
        keybindings: [monaco.KeyMod.CtrlCmd | monaco.KeyMod.Shift | monaco.KeyCode.Enter],
        run: () => onExplainRef.current?.(),
      });
      editor.addAction({
        id: "analyze-query",
        label: "Explain Analyze Statement (executes it)",
        run: () => onAnalyzeRef.current?.(),
      });
    },
    [],
  );

  return (
    <div
      className="relative flex-1 overflow-hidden bg-[var(--color-editor-bg)]"
      suppressHydrationWarning
    >
      <div className="absolute inset-0 overflow-auto bg-editor-bg">
        <Editor
          path={modelPath}
          keepCurrentModel
          height="100%"
          defaultLanguage="pgsql"
          language="pgsql"
          theme={theme === "light" ? "rsql-light" : "rsql-dark"}
          loading={
            <div className="flex h-full w-full items-center justify-center bg-editor-bg">
              <span className="text-muted-foreground text-sm">Loading editor...</span>
            </div>
          }
          options={{
            automaticLayout: true,
            minimap: { enabled: false },
            scrollBeyondLastLine: false,
            fontSize: 13,
            lineNumbers: "on",
            fontFamily: "ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, monospace",
            fontLigatures: false,
            renderLineHighlight: "all",
            padding: { top: 8, bottom: 8 },
            quickSuggestions: {
              other: true,
              comments: false,
              // A literal is not a place to complete identifiers.
              strings: false,
            },
            suggestOnTriggerCharacters: true,
            // The document's own words competed with real schema suggestions.
            wordBasedSuggestions: "off",
            // Enter accepts only when the suggestion would actually change the
            // text, and breaks the line otherwise. Bound to accept
            // unconditionally, every newline typed with the widget open
            // inserted whatever happened to be highlighted; never accepting
            // meant reaching for Tab even mid-word. Tab always accepts.
            acceptSuggestionOnEnter: "smart",
            tabCompletion: "on",
            suggest: {
              showKeywords: true,
              showSnippets: true,
              showFunctions: true,
              showWords: false,
              preview: true,
              filterGraceful: true,
            },
          }}
          value={value}
          onChange={(v) => onChange(v ?? "")}
          onMount={handleMount}
        />
      </div>
    </div>
  );
}

import DataEditor, { type DataEditorRef, GridCellKind } from "@glideapps/glide-data-grid";
import { useRef, useState } from "react";
import { createRoot } from "react-dom/client";
import { CompletionWorkerClient } from "../../src/monaco/completion/worker-client";
import "@glideapps/glide-data-grid/dist/index.css";

const prefix = "SELECT id, email FROM users WHERE id > 10;\n".repeat(1220);
const sql = `${prefix}SELECT u.id FROM users u`;
const caret = { lineNumber: 1221, column: 10 };
const wordRange = { startLineNumber: 1221, endLineNumber: 1221, startColumn: 10, endColumn: 10 };
const percentile = (values: number[], p: number) =>
  [...values].sort((a, b) => a - b)[
    Math.min(values.length - 1, Math.ceil(values.length * p) - 1)
  ] ?? 0;

function Benchmark() {
  const grid = useRef<DataEditorRef>(null);
  const [busy, setBusy] = useState(false);
  const [output, setOutput] = useState(
    "Ready. 50K SQL, 20 warmed samples, concurrent scrolling across 100K rows.",
  );
  async function run(worker: boolean) {
    setBusy(true);
    const client = new CompletionWorkerClient();
    const local = worker ? null : (await import("../baseline-completion")).createBaselineParser();
    const parse = (version: number) => {
      const request = {
        document: "benchmark",
        version,
        sql: `${sql} -- ${version}`,
        caret,
        wordRange,
      };
      return worker ? client.parse(request) : Promise.resolve(local?.(request));
    };
    try {
      for (let i = 0; i < 3; i++) await parse(i);
      const longTasks: number[] = [];
      const observer = new PerformanceObserver((list) =>
        longTasks.push(...list.getEntries().map((entry) => entry.duration)),
      );
      observer.observe({ type: "longtask" });
      const frames: number[] = [];
      let last = performance.now();
      let scrolling = true;
      let row = 0;
      const frame = (now: number) => {
        frames.push(now - last);
        last = now;
        row += 5;
        grid.current?.scrollTo(0, row, "vertical");
        if (scrolling) requestAnimationFrame(frame);
      };
      requestAnimationFrame(frame);
      const samples: number[] = [];
      for (let i = 3; i < 23; i++) {
        await new Promise((resolve) => requestAnimationFrame(resolve));
        const start = performance.now();
        await parse(i);
        samples.push(performance.now() - start);
      }
      await new Promise((resolve) => setTimeout(resolve, 100));
      scrolling = false;
      observer.disconnect();
      setOutput(
        JSON.stringify(
          {
            mode: worker ? "worker" : "main thread",
            sqlCharacters: sql.length,
            samples: samples.length,
            medianMs: percentile(samples, 0.5),
            p95Ms: percentile(samples, 0.95),
            frameP95Ms: percentile(frames.slice(1), 0.95),
            longTasks: longTasks.length,
            longestTaskMs: Math.max(0, ...longTasks),
            userAgent: navigator.userAgent,
          },
          null,
          2,
        ),
      );
    } catch (error) {
      setOutput(String(error));
    } finally {
      client.dispose();
      setBusy(false);
    }
  }
  return (
    <main style={{ fontFamily: "system-ui", padding: 24 }}>
      <h1>Completion and scrolling benchmark</h1>
      <button type="button" disabled={busy} onClick={() => run(false)}>
        Measure main thread
      </button>{" "}
      <button type="button" disabled={busy} onClick={() => run(true)}>
        Measure worker
      </button>
      <pre aria-live="polite">{busy ? "Measuring…" : output}</pre>
      <DataEditor
        ref={grid}
        width={1000}
        height={480}
        rows={100_000}
        columns={[
          { title: "id", width: 160 },
          { title: "value", width: 650 },
        ]}
        getCellContent={([col, row]) => ({
          kind: GridCellKind.Text,
          data: col ? `Row ${row} — Unicode 🦀 and text`.repeat(4) : String(row),
          displayData: String(row),
          allowOverlay: false,
        })}
      />
    </main>
  );
}

createRoot(document.getElementById("root") as HTMLElement).render(<Benchmark />);

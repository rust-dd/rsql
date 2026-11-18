import {
  Check,
  ChevronDown,
  Database,
  FileCode2,
  Loader2,
  Play,
  RotateCcw,
  Table2,
} from "lucide-react";
import { useCallback, useEffect, useRef, useState } from "react";
import type { QueryResult } from "@/hooks/use-sql";
import { useDBReady, useExecuteSQL, useResetDB, useTables } from "@/hooks/use-sql";

const samples = [
  {
    label: "Meet the team",
    sql: "SELECT name, department, salary\nFROM employees\nWHERE is_active = true\nORDER BY name;",
  },
  {
    label: "Department stats",
    sql: "SELECT department,\n       COUNT(*) AS people,\n       ROUND(AVG(salary), 2) AS avg_salary\nFROM employees\nWHERE is_active = true\nGROUP BY department\nORDER BY avg_salary DESC;",
  },
  {
    label: "Active projects",
    sql: "SELECT p.title, d.name AS department, p.status\nFROM projects p\nJOIN departments d ON d.id = p.department_id\nWHERE p.status = 'active'\nORDER BY p.title;",
  },
];

export default function Demo() {
  const database = useDBReady();
  const [sql, setSql] = useState(samples[0].sql);
  const [result, setResult] = useState<QueryResult | null>(null);
  const [resetError, setResetError] = useState<string | null>(null);
  const executeMutation = useExecuteSQL();
  const resetMutation = useResetDB();
  const busyRef = useRef(false);
  const initialized = useRef(false);
  const mounted = useRef(true);
  const busy = executeMutation.isPending || resetMutation.isPending;
  const { mutateAsync: runSQL } = executeMutation;

  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
    };
  }, []);

  const execute = useCallback(
    async (query: string) => {
      if (!query.trim() || busyRef.current) return;
      busyRef.current = true;
      setResetError(null);
      try {
        const nextResult = await runSQL(query);
        if (mounted.current) setResult(nextResult);
      } finally {
        busyRef.current = false;
      }
    },
    [runSQL],
  );

  useEffect(() => {
    if (database.data && !initialized.current) {
      initialized.current = true;
      void execute(samples[0].sql);
    }
  }, [database.data, execute]);

  const reset = async () => {
    if (busyRef.current) return;
    busyRef.current = true;
    setResetError(null);
    try {
      await resetMutation.mutateAsync();
      setSql(samples[0].sql);
      const nextResult = await runSQL(samples[0].sql);
      if (mounted.current) setResult(nextResult);
    } catch (error) {
      setResetError(error instanceof Error ? error.message : "Could not reset the database.");
    } finally {
      busyRef.current = false;
    }
  };

  if (database.isError)
    return (
      <div className="playground-loading" role="alert">
        <p>Couldn’t start the sample database.</p>
        <span>
          {database.error instanceof Error ? database.error.message : "Please try again."}
        </span>
        <button
          type="button"
          className="button button-secondary"
          onClick={() => void database.refetch()}
        >
          Try again
        </button>
      </div>
    );
  if (!database.data)
    return (
      <div className="playground-loading" role="status">
        <Loader2 className="spin" size={24} />
        <p>Starting your sample database…</p>
      </div>
    );

  const chooseQuery = (query: string) => {
    if (busyRef.current) return;
    setSql(query);
    void execute(query);
  };

  return (
    <div className="live-demo">
      <div className="demo-samples">
        <span>TRY A QUERY</span>
        {samples.map((sample) => (
          <button
            type="button"
            key={sample.label}
            disabled={busy}
            aria-pressed={sql === sample.sql}
            onClick={() => chooseQuery(sample.sql)}
          >
            {sample.label}
          </button>
        ))}
        <button
          type="button"
          className="demo-reset"
          disabled={busy}
          onClick={() => void reset()}
          aria-label="Reset sample database"
        >
          <RotateCcw size={13} />
          <span>Reset data</span>
        </button>
      </div>
      <div className="demo-workspace">
        <DemoSidebar onSelect={chooseQuery} disabled={busy} />
        <div className="demo-main">
          <div className="demo-toolbar">
            <span>
              <FileCode2 size={13} /> query.sql
            </span>
            <button
              type="button"
              className="button button-small button-primary"
              disabled={busy || !sql.trim()}
              onClick={() => void execute(sql)}
            >
              {busy ? <Loader2 size={13} className="spin" /> : <Play size={13} />}
              {busy ? "Working…" : "Run query"}
            </button>
          </div>
          <label className="sr-only" htmlFor="sql-editor">
            SQL query
          </label>
          <textarea
            id="sql-editor"
            spellCheck={false}
            value={sql}
            onChange={(event) => setSql(event.target.value)}
            onKeyDown={(event) => {
              if ((event.metaKey || event.ctrlKey) && event.key === "Enter") {
                event.preventDefault();
                void execute(sql);
              }
            }}
            aria-describedby="sql-shortcut"
          />
          <div className="demo-results-heading">
            <span>
              <Table2 size={13} /> Results
            </span>
            <span role="status" aria-live="polite">
              {busy ? (
                "Running…"
              ) : result ? (
                result.error ? (
                  "Query error"
                ) : (
                  <>
                    <Check size={12} />
                    {result.rowCount} rows · {result.time.toFixed(1)} ms
                  </>
                )
              ) : (
                "Ready"
              )}
            </span>
          </div>
          <div className="demo-results" aria-busy={busy}>
            {resetError ? (
              <p className="demo-error" role="alert">
                {resetError}
              </p>
            ) : result ? (
              <Results result={result} />
            ) : (
              <p className="demo-empty">Run a query to see your results.</p>
            )}
          </div>
        </div>
      </div>
      <div className="demo-status">
        <span>
          <span className="status-dot" /> Local sample database
        </span>
        <span id="sql-shortcut">⌘ / Ctrl + Enter to run</span>
      </div>
    </div>
  );
}

function DemoSidebar({
  onSelect,
  disabled,
}: {
  onSelect: (sql: string) => void;
  disabled: boolean;
}) {
  const { data: tables, isError } = useTables();
  return (
    <aside className="demo-sidebar" aria-label="Sample database tables">
      <span className="eyebrow">EXPLORER</span>
      <div>
        <ChevronDown size={12} />
        <Database size={14} />
        <strong>sample_database</strong>
      </div>
      <span className="demo-schema">public</span>
      {isError ? (
        <p>Couldn’t load tables. Try resetting the data.</p>
      ) : (
        tables?.map((table) => (
          <button
            type="button"
            key={table}
            disabled={disabled}
            onClick={() => onSelect(`SELECT * FROM "${table.replaceAll('"', '""')}" LIMIT 100;`)}
          >
            <Table2 size={13} />
            {table}
          </button>
        ))
      )}
    </aside>
  );
}

function Results({ result }: { result: QueryResult }) {
  if (result.error)
    return (
      <pre className="demo-error" role="alert">
        {result.error}
      </pre>
    );
  if (!result.columns.length)
    return <p className="demo-empty">Query executed successfully. No result columns returned.</p>;
  return (
    <table>
      <caption className="sr-only">Query results</caption>
      <thead>
        <tr>
          {result.columns.map((column) => (
            <th scope="col" key={column}>
              {column}
            </th>
          ))}
        </tr>
      </thead>
      <tbody>
        {result.rows.map((row, index) => (
          // biome-ignore lint/suspicious/noArrayIndexKey: SQL rows may be identical; this read-only result snapshot has no row state.
          <tr key={index}>
            {result.columns.map((column) => (
              <td key={column}>{formatCell(row[column])}</td>
            ))}
          </tr>
        ))}
      </tbody>
    </table>
  );
}

function formatCell(value: unknown): string {
  if (value === null || value === undefined) return "NULL";
  if (typeof value === "object") return JSON.stringify(value);
  return String(value);
}

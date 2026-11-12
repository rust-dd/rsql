import {
  ChevronDown,
  ChevronRight,
  Columns3,
  Database,
  FolderOpen,
  Key,
  Loader2,
  Play,
  RotateCcw,
  Server,
  Table,
} from "lucide-react";
import { useCallback, useEffect, useRef, useState } from "react";
import type { QueryResult } from "@/hooks/use-sql";
import { useColumns, useDBReady, useExecuteSQL, useResetDB, useTables } from "@/hooks/use-sql";

const SAMPLE_QUERIES: { label: string; sql: string }[] = [
  {
    label: "All Employees",
    sql: "SELECT * FROM employees ORDER BY name;",
  },
  {
    label: "Department Stats",
    sql: `SELECT department,
       COUNT(*) AS headcount,
       ROUND(AVG(salary), 2) AS avg_salary
FROM employees
WHERE is_active = true
GROUP BY department
ORDER BY avg_salary DESC;`,
  },
  {
    label: "Top Salaries",
    sql: `SELECT name, department, salary
FROM employees
ORDER BY salary DESC
LIMIT 5;`,
  },
  {
    label: "Active Projects",
    sql: `SELECT p.title, d.name AS department, p.status, p.start_date
FROM projects p
JOIN departments d ON d.id = p.department_id
WHERE p.status = 'active'
ORDER BY p.start_date;`,
  },
  {
    label: "Budget Overview",
    sql: `SELECT d.name,
       d.budget,
       d.location,
       COUNT(p.id) AS project_count
FROM departments d
LEFT JOIN projects p ON p.department_id = d.id
GROUP BY d.id, d.name, d.budget, d.location
ORDER BY d.budget DESC;`,
  },
];

export function Demo() {
  const { data: ready } = useDBReady();
  const [sql, setSql] = useState(SAMPLE_QUERIES[0].sql);
  const [result, setResult] = useState<QueryResult | null>(null);
  const textareaRef = useRef<HTMLTextAreaElement>(null);
  const executeMutation = useExecuteSQL();
  const resetMutation = useResetDB();

  const execute = useCallback(() => {
    if (!sql.trim()) return;
    executeMutation.mutate(sql, {
      onSuccess: (data) => setResult(data),
    });
  }, [sql, executeMutation]);

  const handleKeyDown = (e: React.KeyboardEvent) => {
    if ((e.metaKey || e.ctrlKey) && e.key === "Enter") {
      e.preventDefault();
      execute();
    }
  };

  useEffect(() => {
    if (ready && !result) {
      executeMutation.mutate(SAMPLE_QUERIES[0].sql, {
        onSuccess: (data) => setResult(data),
      });
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [ready, result, executeMutation.mutate]);

  if (!ready) {
    return (
      <section className="px-6 py-24 text-center" id="demo">
        <div className="mx-auto max-w-[1080px]">
          <Loader2 className="h-6 w-6 animate-spin text-[var(--accent)] mx-auto mb-3" />
          <p className="text-sm text-[var(--fg-muted)]">Starting PostgreSQL…</p>
          <p className="text-xs text-[var(--fg-subtle)] mt-1 font-[var(--font-mono)]">
            PGlite WebAssembly engine
          </p>
        </div>
      </section>
    );
  }

  return (
    <section className="px-6 py-24" id="demo">
      <div className="mx-auto max-w-[1080px]">
        <div className="divider mb-16" />

        <div className="mb-10 max-w-lg">
          <span className="section-label">Sandbox</span>
          <h2 className="font-display text-[clamp(1.8rem,4vw,2.8rem)] mt-3">Try it right here</h2>
          <p className="text-[var(--fg-muted)] mt-3 text-[15px] leading-relaxed">
            Full PostgreSQL running in WebAssembly. Real SQL, seeded data, no install.
          </p>
        </div>

        {/* Query pills */}
        <div className="mb-4 flex flex-wrap gap-2">
          {SAMPLE_QUERIES.map((q) => (
            <button
              key={q.label}
              type="button"
              onClick={() => {
                setSql(q.sql);
                executeMutation.mutate(q.sql, {
                  onSuccess: (data) => setResult(data),
                });
              }}
              className={`rounded-full border px-4 py-2 text-xs font-[var(--font-mono)] transition-colors ${
                sql === q.sql
                  ? "border-[var(--accent)] bg-[var(--accent-muted)] text-[var(--accent)]"
                  : "border-[var(--border)] text-[var(--fg-muted)] hover:text-[var(--fg)] hover:border-[var(--fg-subtle)]"
              }`}
            >
              {q.label}
            </button>
          ))}
        </div>

        {/* Sandbox */}
        <div className="product-frame">
          <div className="product-frame-titlebar">
            <div className="product-frame-dot bg-[#ff5f57]" />
            <div className="product-frame-dot bg-[#febc2e]" />
            <div className="product-frame-dot bg-[#28c840]" />
            <span className="flex-1 text-center text-[11px] text-[var(--fg-subtle)] font-[var(--font-mono)]">
              RSQL — browser sandbox
            </span>
            <button
              type="button"
              onClick={() => resetMutation.mutate()}
              className="text-[var(--fg-subtle)] hover:text-[var(--fg-muted)] transition-colors"
              title="Reset database"
            >
              <RotateCcw className="h-3.5 w-3.5" />
            </button>
          </div>

          <div className="flex h-[480px]">
            <DemoSidebar
              onTableClick={(table) => {
                const q = `SELECT * FROM ${table} LIMIT 100;`;
                setSql(q);
                executeMutation.mutate(q, {
                  onSuccess: (data) => setResult(data),
                });
              }}
            />

            <div className="flex-1 flex flex-col min-w-0">
              {/* Toolbar */}
              <div className="flex items-center gap-2 px-3 py-2 border-b border-[var(--border-subtle)] bg-[var(--surface-raised)]">
                <button
                  type="button"
                  onClick={execute}
                  disabled={executeMutation.isPending}
                  className="flex items-center gap-1.5 rounded-lg bg-[var(--accent)] px-3 py-1 text-xs font-semibold text-white disabled:opacity-50"
                >
                  {executeMutation.isPending ? (
                    <Loader2 className="h-3 w-3 animate-spin" />
                  ) : (
                    <Play className="h-3 w-3" />
                  )}
                  Run
                </button>
                <span className="text-[10px] text-[var(--fg-subtle)] font-[var(--font-mono)] ml-auto">
                  {result
                    ? result.error
                      ? "Error"
                      : `${result.rowCount} rows · ${result.time.toFixed(1)}ms`
                    : "⌘+Enter"}
                </span>
              </div>

              {/* Editor */}
              <div className="relative flex-none h-[180px] border-b border-[var(--border-subtle)]">
                <textarea
                  ref={textareaRef}
                  value={sql}
                  onChange={(e) => setSql(e.target.value)}
                  onKeyDown={handleKeyDown}
                  spellCheck={false}
                  className="w-full h-full resize-none editor-bg text-[var(--fg)] font-[var(--font-mono)] text-[13px] leading-6 p-4 focus:outline-hidden"
                  placeholder="Write SQL here..."
                />
              </div>

              {/* Results */}
              <div className="flex-1 overflow-auto">
                {result && <DemoResults result={result} />}
              </div>
            </div>
          </div>
        </div>
      </div>
    </section>
  );
}

/* ─── Sidebar ─── */

function DemoSidebar({ onTableClick }: { onTableClick: (table: string) => void }) {
  const { data: tableList } = useTables();
  const [expandedTable, setExpandedTable] = useState<string | null>(null);
  const [serverOpen, setServerOpen] = useState(true);
  const [schemaOpen, setSchemaOpen] = useState(true);

  return (
    <div className="w-[200px] flex-none border-r border-[var(--border-subtle)] bg-[var(--surface-raised)] overflow-y-auto text-xs select-none">
      <div className="px-3 py-2.5 border-b border-[var(--border-subtle)]">
        <span className="text-[10px] font-semibold text-[var(--fg-subtle)] tracking-widest uppercase font-[var(--font-mono)]">
          Navigator
        </span>
      </div>

      <button
        type="button"
        onClick={() => setServerOpen(!serverOpen)}
        className="surface-hover flex items-center gap-1.5 w-full px-3 py-1.5 text-left"
      >
        {serverOpen ? (
          <ChevronDown className="h-3 w-3 text-[var(--fg-subtle)]" />
        ) : (
          <ChevronRight className="h-3 w-3 text-[var(--fg-subtle)]" />
        )}
        <Server className="h-3 w-3 text-[var(--accent)]" />
        <span className="font-[var(--font-mono)] font-semibold text-xs">local</span>
        <span className="ml-auto h-1.5 w-1.5 rounded-full bg-[var(--success)]" />
      </button>

      {serverOpen && (
        <>
          <div className="flex items-center gap-1.5 px-3 py-1 pl-6 text-[var(--fg-subtle)]">
            <ChevronDown className="h-3 w-3" />
            <Database className="h-3 w-3" />
            <span className="font-[var(--font-mono)] text-[11px]">catalog</span>
          </div>

          <button
            type="button"
            onClick={() => setSchemaOpen(!schemaOpen)}
            className="surface-hover flex items-center gap-1.5 w-full px-3 py-1 pl-9 text-left"
          >
            {schemaOpen ? (
              <ChevronDown className="h-3 w-3 text-[var(--fg-subtle)]" />
            ) : (
              <ChevronRight className="h-3 w-3 text-[var(--fg-subtle)]" />
            )}
            <Database className="h-3 w-3 text-[var(--fg-subtle)]" />
            <span className="font-[var(--font-mono)] text-[11px]">demo</span>
            <span className="ml-auto h-1.5 w-1.5 rounded-full bg-[var(--success)]" />
          </button>

          {schemaOpen && (
            <>
              <div className="flex items-center gap-1.5 px-3 py-0.5 pl-12 text-[var(--fg-subtle)]">
                <ChevronDown className="h-3 w-3" />
                <FolderOpen className="h-3 w-3" />
                <span className="font-[var(--font-mono)] text-[11px]">public</span>
              </div>

              {tableList?.map((table) => (
                <TableNode
                  key={table}
                  table={table}
                  expanded={expandedTable === table}
                  onToggle={() => setExpandedTable(expandedTable === table ? null : table)}
                  onClick={() => onTableClick(table)}
                />
              ))}
            </>
          )}
        </>
      )}
    </div>
  );
}

function TableNode({
  table,
  expanded,
  onToggle,
  onClick,
}: {
  table: string;
  expanded: boolean;
  onToggle: () => void;
  onClick: () => void;
}) {
  const { data: columns } = useColumns(expanded ? table : null);

  return (
    <div>
      <button
        type="button"
        className="surface-hover flex items-center gap-1.5 w-full px-3 py-0.5 pl-[56px] text-left"
        onClick={onToggle}
        onDoubleClick={onClick}
      >
        {expanded ? (
          <ChevronDown className="h-3 w-3 text-[var(--fg-subtle)]" />
        ) : (
          <ChevronRight className="h-3 w-3 text-[var(--fg-subtle)]" />
        )}
        <Table className="h-3 w-3 text-[var(--fg-subtle)]" />
        <span className="font-[var(--font-mono)] text-[11px]">{table}</span>
      </button>
      {expanded && columns && (
        <div>
          {columns.map((col) => (
            <div
              key={col.column_name}
              className="flex items-center gap-1.5 px-3 py-0 pl-[76px] text-[var(--fg-subtle)]"
            >
              {col.column_name === "id" ? (
                <Key className="h-2.5 w-2.5 text-amber-400/60" />
              ) : (
                <Columns3 className="h-2.5 w-2.5 opacity-30" />
              )}
              <span className="font-[var(--font-mono)] text-[10px]">{col.column_name}</span>
              <span className="font-[var(--font-mono)] text-[9px] opacity-40">{col.data_type}</span>
            </div>
          ))}
        </div>
      )}
    </div>
  );
}

function DemoResults({ result }: { result: QueryResult }) {
  if (result.error) {
    return (
      <div className="p-4">
        <pre className="font-[var(--font-mono)] text-xs text-[var(--destructive)] whitespace-pre-wrap">
          {result.error}
        </pre>
      </div>
    );
  }

  if (!result.columns.length) {
    return (
      <div className="p-4 text-xs text-[var(--fg-muted)] font-[var(--font-mono)]">
        Query executed successfully. No rows returned.
      </div>
    );
  }

  return (
    <table className="w-full text-left font-[var(--font-mono)] text-[12px] border-collapse">
      <thead>
        <tr>
          {result.columns.map((col) => (
            <th
              key={col}
              className="sticky top-0 z-10 border-b border-r last:border-r-0 border-[var(--border-subtle)] bg-[var(--surface-raised)] px-3 py-1.5 text-[10px] font-semibold whitespace-nowrap text-[var(--fg-muted)]"
            >
              {col}
            </th>
          ))}
        </tr>
      </thead>
      <tbody>
        {result.rows.map((row, i) => (
          <tr key={i} className="hover:bg-[var(--accent-muted)] transition-colors">
            {result.columns.map((col) => (
              <td
                key={col}
                className="px-3 py-1 text-[var(--fg-muted)] border-b border-r last:border-r-0 border-[var(--border-subtle)] whitespace-nowrap"
              >
                {formatCell(row[col])}
              </td>
            ))}
          </tr>
        ))}
      </tbody>
    </table>
  );
}

function formatCell(val: unknown): string {
  if (val === null || val === undefined) return "NULL";
  if (typeof val === "boolean") return val ? "true" : "false";
  return String(val);
}

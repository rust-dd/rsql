import {
  ArrowDown,
  ArrowUpRight,
  ChevronDown,
  ChevronRight,
  Database,
  FileCode2,
  GitBranch,
  KeyRound,
  Layers,
  Search,
  Table2,
} from "lucide-react";
import { useState } from "react";
import { SqlCode } from "@/components/SqlCode";

const views = ["Query", "Explore", "Explain"] as const;
const sql = `-- A little clarity, one query at a time.\nSELECT department,\n       COUNT(*) AS people,\n       ROUND(AVG(salary), 2) AS avg_salary\nFROM employees\nWHERE is_active = true\nGROUP BY department\nORDER BY avg_salary DESC;`;
const rows = [
  ["Engineering", "4", "149500.00"],
  ["Data Science", "2", "146000.00"],
  ["Design", "2", "101500.00"],
  ["Marketing", "2", "89500.00"],
  ["Sales", "1", "88000.00"],
];

export function ProductPreview() {
  const [view, setView] = useState<(typeof views)[number]>("Query");
  return (
    <div className="product-stage">
      <div className="product-window">
        <div className="window-titlebar">
          <div className="window-dots" aria-hidden="true">
            <i />
            <i />
            <i />
          </div>
          <span>
            <Database size={13} /> RSQL
          </span>
          <span className="preview-label">PRODUCT PREVIEW</span>
        </div>
        <div className="product-body">
          <aside className="product-sidebar" aria-label="Example database schema">
            <div className="sidebar-heading">
              EXPLORER
              <Layers size={13} />
            </div>
            <div className="preview-connection">
              <ChevronDown size={13} />
              <Database size={15} />
              <strong>workspace</strong>
              <span className="status-dot" />
            </div>
            <div className="sidebar-schema">
              <ChevronDown size={12} /> public
            </div>
            {["departments", "employees", "projects"].map((table) => (
              <div
                key={table}
                className={`sidebar-table ${table === "employees" ? "selected" : ""}`}
              >
                <ChevronRight size={12} />
                <Table2 size={13} />
                {table}
              </div>
            ))}
            <div className="sidebar-columns">
              {["id", "name", "department", "salary", "hire_date", "is_active"].map((column) => (
                <div key={column}>
                  {column === "id" ? <KeyRound size={11} /> : <span className="column-dot" />}
                  {column}
                </div>
              ))}
            </div>
            <div className="sidebar-bottom">
              <span className="status-dot" /> PostgreSQL / localhost
            </div>
          </aside>
          <div className="product-content">
            <div className="preview-toolbar">
              <span>
                <FileCode2 size={13} /> team_overview.sql
              </span>
              <a href="#demo">
                Try a query <ArrowUpRight size={13} />
              </a>
            </div>
            <div className="preview-view-selector">
              {views.map((item) => (
                <button
                  key={item}
                  type="button"
                  aria-pressed={view === item}
                  onClick={() => setView(item)}
                >
                  {item === "Query" ? (
                    <FileCode2 size={13} />
                  ) : item === "Explore" ? (
                    <Table2 size={13} />
                  ) : (
                    <GitBranch size={13} />
                  )}
                  {item}
                </button>
              ))}
            </div>
            {view === "Query" ? (
              <>
                <div className="preview-editor">
                  <div className="line-numbers" aria-hidden="true">
                    {sql.split("\n").map((line, index) => (
                      <span key={line}>{index + 1}</span>
                    ))}
                  </div>
                  <pre>
                    <SqlCode sql={sql} />
                  </pre>
                </div>
                <div className="preview-result-bar">
                  <span>
                    <Table2 size={13} /> Results <span className="count-badge">5</span>
                  </span>
                  <span className="preview-result-hint">
                    <span className="status-dot" /> Example result
                  </span>
                </div>
                <div className="preview-table-wrap">
                  <table className="preview-table">
                    <thead>
                      <tr>
                        <th>#</th>
                        <th>
                          department <span>text</span>
                        </th>
                        <th>
                          people <span>int8</span>
                        </th>
                        <th>
                          avg_salary <ArrowDown size={10} />
                          <span>numeric</span>
                        </th>
                      </tr>
                    </thead>
                    <tbody>
                      {rows.map((row, index) => (
                        <tr key={row[0]}>
                          <td>{index + 1}</td>
                          {row.map((value) => (
                            <td key={value}>{value}</td>
                          ))}
                        </tr>
                      ))}
                    </tbody>
                  </table>
                </div>
              </>
            ) : view === "Explore" ? (
              <SchemaPreview />
            ) : (
              <ExplainPreview />
            )}
            <div className="preview-status">
              <span>
                <span className="status-dot" /> Connected
              </span>
              <span>
                UTF-8 <span className="status-separator">|</span> PostgreSQL
              </span>
            </div>
          </div>
        </div>
      </div>
      <div className="product-stage-note">
        <span>One place for the whole picture.</span>
        <span>Query. Explore. Understand.</span>
      </div>
    </div>
  );
}

function SchemaPreview() {
  return (
    <div className="schema-preview">
      <div className="preview-view-heading">
        <Search size={17} />
        <div>
          <h3>Know your database.</h3>
          <p>Tables, columns, and the connections between them.</p>
        </div>
      </div>
      <div className="schema-cards">
        {[
          {
            name: "departments",
            fields: [
              ["id", "integer · PK"],
              ["name", "text"],
              ["budget", "numeric"],
              ["location", "text"],
            ],
          },
          {
            name: "projects",
            fields: [
              ["id", "integer · PK"],
              ["department_id", "integer · FK"],
              ["title", "text"],
              ["status", "text"],
            ],
          },
        ].map((table) => (
          <div key={table.name} className="schema-card">
            <strong>
              <Table2 size={14} />
              {table.name}
            </strong>
            {table.fields.map(([field, type]) => (
              <div key={field}>
                <span>{field}</span>
                <small>{type}</small>
              </div>
            ))}
          </div>
        ))}
      </div>
      <div className="schema-relationship">
        <GitBranch size={14} /> projects.department_id → departments.id
      </div>
      <p className="preview-view-note">
        Inspect structure. Follow foreign keys. Keep your context.
      </p>
    </div>
  );
}

function ExplainPreview() {
  return (
    <div className="explain-preview">
      <div className="preview-view-heading">
        <GitBranch size={17} />
        <div>
          <h3>Find the expensive part.</h3>
          <p>Follow the plan, from the first scan to the last sort.</p>
        </div>
      </div>
      <div className="plan-summary">
        <span>EXAMPLE QUERY PLAN</span>
        <strong>EXPLAIN ANALYZE</strong>
      </div>
      <div className="plan-tree">
        {[
          { name: "Sort", depth: 0, width: "34%", detail: "avg_salary DESC" },
          { name: "HashAggregate", depth: 1, width: "58%", detail: "Group key: department" },
          {
            name: "Seq Scan on employees",
            depth: 2,
            width: "82%",
            detail: "Filter: is_active = true",
          },
        ].map((node) => (
          <div key={node.name} className="plan-node" style={{ marginLeft: node.depth * 16 }}>
            <div>
              <ChevronDown size={12} />
              <strong>{node.name}</strong>
              <span>{node.detail}</span>
            </div>
            <div className="plan-track">
              <span style={{ width: node.width }} />
            </div>
          </div>
        ))}
      </div>
      <p className="preview-view-note">Inspect timing, costs, and estimated versus actual rows.</p>
    </div>
  );
}

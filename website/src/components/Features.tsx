import {
  ArrowDownToLine,
  ArrowUpRight,
  Braces,
  Check,
  Columns2,
  Command,
  FileCode2,
  GitBranch,
  History,
  Table2,
} from "lucide-react";
import { SqlCode } from "@/components/SqlCode";
import { DOCS_URL } from "@/lib/site";

export function Features() {
  return (
    <section id="features" className="section container" aria-labelledby="features-heading">
      <div className="section-heading">
        <div>
          <span className="eyebrow">01 / THE WORKBENCH</span>
          <h2 id="features-heading">
            Everything in reach.
            <br />
            <span>Nothing in your way.</span>
          </h2>
        </div>
        <p>
          From a quick lookup to a deep dive.
          <br />
          The tools you need, right where you need them.
        </p>
      </div>
      <div className="feature-grid">
        <article className="feature-card feature-editor">
          <div className="feature-copy">
            <span className="feature-icon">
              <FileCode2 size={20} />
            </span>
            <h3>Find your flow.</h3>
            <p>
              Schema-aware completion, saved queries, and shortcuts that keep your hands on the
              keyboard.
            </p>
          </div>
          <div className="editor-illustration">
            <div>
              <span className="tab-file">
                <FileCode2 size={12} /> a_good_place_to_start.sql
              </span>
              <span>SQL</span>
            </div>
            <pre>
              <SqlCode sql={"SELECT name, department\nFROM employees\nWHERE is_"} />
              <span className="typing-cursor" />
            </pre>
            <div className="completion-popup">
              <div>
                <Braces size={13} />
                <strong>is_active</strong>
                <small>boolean</small>
              </div>
              <div>
                <Table2 size={13} />
                <span>employees</span>
                <small>table</small>
              </div>
              <footer>
                <span>Knows your schema. Saves your keystrokes.</span>
                <kbd>↵</kbd>
              </footer>
            </div>
          </div>
          <div className="feature-footnote">
            <Command size={13} />
            <span>A shortcut from thought to query.</span>
          </div>
        </article>
        <article className="feature-card feature-data">
          <div className="feature-copy">
            <span className="feature-icon">
              <Table2 size={20} />
            </span>
            <h3>Get closer to your data.</h3>
            <p>
              Browse results, inspect records, and edit eligible rows. Apply your changes together
              when you’re ready.
            </p>
          </div>
          <div className="data-illustration">
            <div className="mini-table">
              <div className="mini-table-header">
                <span>name</span>
                <span>department</span>
              </div>
              <div>
                <span>Alice Chen</span>
                <span>Engineering</span>
              </div>
              <div>
                <span>Eve Davis</span>
                <span className="edited-cell">
                  Design <Check size={12} />
                </span>
              </div>
              <div>
                <span>Grace Lee</span>
                <span>Data Science</span>
              </div>
            </div>
            <div className="edit-confirmation">
              <span className="status-dot" />1 pending change
              <span>
                <Check size={12} /> Apply changes
              </span>
            </div>
          </div>
        </article>
        <article className="feature-card feature-understand">
          <div className="feature-copy">
            <span className="feature-icon">
              <GitBranch size={20} />
            </span>
            <h3>See the bigger picture.</h3>
            <p>
              Explore schemas, follow relationships, and inspect query plans without losing your
              place.
            </p>
          </div>
          <div className="relationship-illustration">
            <div>
              <Table2 size={14} />
              <strong>departments</strong>
              <small>
                id <KeyLabel />
              </small>
            </div>
            <span className="relationship-line">
              <i />
              <i />
            </span>
            <div>
              <Table2 size={14} />
              <strong>projects</strong>
              <small>
                department_id <span>FK</span>
              </small>
            </div>
          </div>
          <a className="text-link" href={DOCS_URL}>
            Explore the workbench <ArrowUpRight size={15} />
          </a>
        </article>
      </div>
      <div className="feature-extras">
        {[
          {
            icon: Columns2,
            title: "Room to think",
            text: "Tabs, split views, and saved workspaces.",
          },
          {
            icon: History,
            title: "Pick up where you left off",
            text: "Query history and saved SQL, close at hand.",
          },
          {
            icon: ArrowDownToLine,
            title: "Take your work with you",
            text: "Import CSV. Export the results you need.",
          },
        ].map(({ icon: Icon, title, text }) => (
          <div key={title}>
            <Icon size={20} />
            <div>
              <h3>{title}</h3>
              <p>{text}</p>
            </div>
          </div>
        ))}
      </div>
    </section>
  );
}

function KeyLabel() {
  return <span>PK</span>;
}

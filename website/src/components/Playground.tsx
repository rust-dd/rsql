import { ArrowRight, Database, Loader2, Play, Terminal } from "lucide-react";
import type { ReactNode } from "react";
import { Component, lazy, Suspense, useState } from "react";
import { SqlCode } from "@/components/SqlCode";

const Demo = lazy(() => import("@/components/Demo"));

class PlaygroundErrorBoundary extends Component<{ children: ReactNode }, { failed: boolean }> {
  state = { failed: false };

  static getDerivedStateFromError() {
    return { failed: true };
  }

  render() {
    if (this.state.failed)
      return (
        <div className="playground-loading" role="alert">
          <p>The playground couldn’t load. Check your connection and reload to try again.</p>
          <button
            type="button"
            className="button button-secondary"
            onClick={() => window.location.reload()}
          >
            Reload playground
          </button>
        </div>
      );
    return this.props.children;
  }
}

export function Playground() {
  const [started, setStarted] = useState(false);
  return (
    <section id="demo" className="playground-section" aria-labelledby="playground-heading">
      <div className="container">
        <div className="section-heading">
          <div>
            <span className="eyebrow">02 / TAKE IT FOR A SPIN</span>
            <h2 id="playground-heading">
              Less reading.
              <br />
              <span>More querying.</span>
            </h2>
          </div>
          <div>
            <p>
              A real SQL playground, right here.
              <br />
              Sample data included. Curiosity encouraged.
            </p>
            <span className="playground-local">
              <span className="status-dot" /> Runs locally in your browser
            </span>
          </div>
        </div>
        <div className="playground-shell">
          <div className="playground-titlebar">
            <span>
              <Terminal size={16} /> THE PLAYGROUND
            </span>
            <span>
              <Database size={12} /> PGlite / PostgreSQL
            </span>
          </div>
          {started ? (
            <PlaygroundErrorBoundary>
              <Suspense
                fallback={
                  <div className="playground-loading" role="status">
                    <Loader2 className="spin" size={24} />
                    <p>Loading your SQL playground…</p>
                  </div>
                }
              >
                <Demo />
              </Suspense>
            </PlaygroundErrorBoundary>
          ) : (
            <div className="playground-start">
              <div className="playground-start-code">
                <div>
                  <span>01</span>
                  <SqlCode sql="SELECT name, department" />
                </div>
                <div>
                  <span>02</span>
                  <SqlCode sql="FROM employees" />
                </div>
                <div>
                  <span>03</span>
                  <SqlCode sql="WHERE is_active = true;" />
                </div>
                <p>12 people. 5 departments. Plenty to explore.</p>
              </div>
              <div className="playground-start-action">
                <span className="play-icon">
                  <Play size={25} />
                </span>
                <h3>Your first query is on us.</h3>
                <p>
                  Start a fresh database with sample data.
                  <br />
                  No setup or signup.
                </p>
                <button
                  className="button button-primary"
                  type="button"
                  onClick={() => setStarted(true)}
                >
                  Launch playground
                  <ArrowRight size={16} />
                </button>
              </div>
            </div>
          )}
        </div>
        <p className="playground-caption">
          A browser sandbox with sample data. Download the desktop app to connect to your own
          databases.
        </p>
      </div>
    </section>
  );
}

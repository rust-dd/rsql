import { ArrowUpRight, Code2, Database, Fingerprint, Layers, Monitor, Zap } from "lucide-react";
import { REPOSITORY_URL } from "@/lib/site";

export function Architecture() {
  return (
    <section id="principles" className="principles" aria-labelledby="principles-heading">
      <div className="container">
        <div className="principles-top">
          <div>
            <span className="eyebrow">03 / SMALL FOOTPRINT. CLEAR PRINCIPLES.</span>
            <h2 id="principles-heading">
              Your database.
              <br />
              Your machine.
              <br />
              <span>Your rules.</span>
            </h2>
            <p>
              A native desktop app written in Rust, with no WebView, and a simple idea: your database
              tools should work for you.
            </p>
            <a className="text-link" href={REPOSITORY_URL}>
              See how it’s built <ArrowUpRight size={16} />
            </a>
          </div>
          <div className="connection-diagram">
            <div className="diagram-grid" aria-hidden="true" />
            <div className="diagram-device">
              <Monitor size={30} />
              <span>YOUR MACHINE</span>
              <strong>RSQL</strong>
              <small>Rust + GPUI</small>
            </div>
            <div className="diagram-path">
              <span />
              <small>DIRECT CONNECTION</small>
              <span />
            </div>
            <div className="diagram-database">
              <Database size={29} />
              <div>
                <strong>PostgreSQL</strong>
                <span>Your database. Wherever it lives.</span>
              </div>
              <span className="status-dot" />
            </div>
            <div className="diagram-note">
              <Fingerprint size={14} /> No RSQL account. No telemetry.
            </div>
          </div>
        </div>
        <div className="principle-grid">
          {[
            {
              icon: Zap,
              title: "A focused foundation",
              text: "Rust from the database driver to the GPU-rendered UI. No WebView. Purpose-built for PostgreSQL.",
            },
            {
              icon: Layers,
              title: "Room for real results",
              text: "A native grid and paged results keep large datasets practical to explore, with explicit memory limits.",
            },
            {
              icon: Code2,
              title: "Open from the start",
              text: "Free to use, inspect, and improve. The full source is available under the MIT license.",
            },
          ].map(({ icon: Icon, title, text }) => (
            <article key={title}>
              <Icon size={21} />
              <h3>{title}</h3>
              <p>{text}</p>
            </article>
          ))}
        </div>
      </div>
    </section>
  );
}

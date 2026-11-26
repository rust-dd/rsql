import { ArrowDown, ArrowUpRight, Download, Play } from "lucide-react";
import { ProductPreview } from "@/components/ProductPreview";
import { DOWNLOAD_URL, downloadLabel, REPOSITORY_URL } from "@/lib/site";

export function Hero() {
  return (
    <section className="hero container" aria-labelledby="hero-heading">
      <div className="hero-intro">
        <a className="eyebrow hero-badge" href={REPOSITORY_URL}>
          <span className="status-dot" /> OPEN SOURCE. POSTGRESQL FOCUSED.{" "}
          <ArrowUpRight size={13} />
        </a>
        <h1 id="hero-heading">
          Less friction.
          <br />
          More <span>SQL.</span>
          <span className="hero-cursor" aria-hidden="true">
            _
          </span>
        </h1>
        <p>
          A calmer place to work with your database.
          <br className="desktop-break" /> Write queries, explore your data, and get back to
          building.
        </p>
        <div className="hero-buttons">
          <a className="button button-primary" href={DOWNLOAD_URL}>
            <Download size={17} />
            {downloadLabel()}
            <ArrowUpRight size={16} />
          </a>
          <a className="button button-secondary" href="#demo">
            <Play size={15} /> Try in your browser
          </a>
        </div>
        <div className="hero-meta">
          <span>macOS</span>
          <span>Windows</span>
          <span>Linux</span>
          <span className="meta-divider" />
          <span>Free. No account needed.</span>
        </div>
      </div>
      <div className="preview-caption">
        <span>
          <span className="status-dot" /> YOUR NEXT QUERY STARTS HERE
        </span>
        <a href="#features">
          MEET YOUR WORKBENCH <ArrowDown size={13} />
        </a>
      </div>
      <ProductPreview />
      <div className="hero-footnote">
        <span>
          Made for PostgreSQL.
          <br />
          <strong>Built around your workflow.</strong>
        </span>
        <div>
          <span>RUST + GPUI</span>
          <span>MIT LICENSED</span>
          <span>NO TELEMETRY</span>
        </div>
      </div>
    </section>
  );
}

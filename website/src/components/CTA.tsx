import { ArrowUpRight, Download } from "lucide-react";
import { GitHubIcon } from "@/components/GitHubIcon";
import { DOWNLOAD_URL, REPOSITORY_URL } from "@/lib/site";

export function CTA() {
  return (
    <section
      id="download"
      className="download-section container"
      aria-labelledby="download-heading"
    >
      <div className="download-panel">
        <div className="download-art" aria-hidden="true">
          <span>SELECT</span>
          <span>something</span>
          <span>
            great<span className="download-semicolon">;</span>
          </span>
        </div>
        <div className="download-copy">
          <span className="eyebrow">LESS SETUP. MORE DOING.</span>
          <h2 id="download-heading">
            Make room for
            <br />
            your next idea.
          </h2>
          <p>Your database is waiting. Let’s get to work.</p>
          <div className="download-buttons">
            <a className="button button-primary" href={DOWNLOAD_URL}>
              <Download size={16} />
              Download RSQL
              <ArrowUpRight size={15} />
            </a>
            <a className="button button-ghost" href={REPOSITORY_URL}>
              <GitHubIcon size={17} />
              View source
            </a>
          </div>
          <span className="download-platforms">
            macOS · Windows · Linux &nbsp; / &nbsp; Free & open source
          </span>
        </div>
      </div>
    </section>
  );
}

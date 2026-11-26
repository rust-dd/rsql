import { ArrowUpRight, Plus } from "lucide-react";
import { REPOSITORY_URL } from "@/lib/site";

const questions = [
  {
    question: "Is RSQL really free?",
    answer:
      "Yes. RSQL is free and open source under the MIT license. There is no subscription, trial period, or account required to use the desktop app.",
  },
  {
    question: "Which databases can I connect to?",
    answer:
      "RSQL is built for PostgreSQL. Connect to a local server or a hosted PostgreSQL database using connection details or a connection URL. SSL and SSH tunnels are supported.",
  },
  {
    question: "Does it work on my operating system?",
    answer:
      "RSQL is available for macOS on Apple Silicon and Intel, Windows on x64, and Linux on x86_64. The releases page includes the installers for each platform.",
  },
  {
    question: "Is the playground the full desktop app?",
    answer:
      "The playground is a small SQL sandbox powered by PGlite, running in your browser with sample data. It doesn’t connect to your databases or include every desktop feature. Download RSQL for the full workbench.",
  },
  {
    question: "Where does RSQL store my data?",
    answer:
      "Saved connections, queries, and workspaces stay on your machine. RSQL connects to your database directly and sends no telemetry. Database and SSH passwords are currently stored unencrypted in the local rsql.db file; treat that file and its backups as sensitive.",
  },
  {
    question: "Can I work with large query results?",
    answer:
      "Yes. RSQL loads result pages as you scroll through the results grid. Each result retains up to one million rows within a 256 MiB budget shared by cached results and running paged queries. Results that reach a limit are marked as truncated.",
  },
];

export function FAQ() {
  return (
    <section id="faq" className="section container faq-section" aria-labelledby="faq-heading">
      <div className="faq-intro">
        <span className="eyebrow">04 / GOOD QUESTIONS</span>
        <h2 id="faq-heading">
          A few things
          <br />
          worth knowing.
        </h2>
        <p>Something else on your mind?</p>
        <a className="text-link" href={`${REPOSITORY_URL}/issues`}>
          Start a conversation on GitHub <ArrowUpRight size={15} />
        </a>
      </div>
      <div className="faq-list">
        {questions.map((item) => (
          <details key={item.question}>
            <summary>
              {item.question}
              <Plus size={18} aria-hidden="true" />
            </summary>
            <p>{item.answer}</p>
          </details>
        ))}
      </div>
    </section>
  );
}

import { ArrowUpRight, Download, Github, Menu, Moon, Sun, X } from "lucide-react";
import { useEffect, useState } from "react";
import { Architecture } from "@/components/Architecture";
import { CTA } from "@/components/CTA";
import { FAQ } from "@/components/FAQ";
import { Features } from "@/components/Features";
import { Hero } from "@/components/Hero";
import { Logo } from "@/components/Logo";
import { Playground } from "@/components/Playground";
import { DOCS_URL, DOWNLOAD_URL, REPOSITORY_URL, THEME_KEY } from "@/lib/site";

const navigation = [
  { href: "#features", label: "The workbench" },
  { href: "#demo", label: "Playground" },
  { href: "#principles", label: "Why RSQL" },
  { href: "#faq", label: "FAQ" },
];

export function App() {
  const [theme, setTheme] = useState(() =>
    document.documentElement.dataset.theme === "dark" ? "dark" : "light",
  );
  const [menuOpen, setMenuOpen] = useState(false);

  useEffect(() => {
    document.documentElement.dataset.theme = theme;
    document.documentElement.style.colorScheme = theme;
    try {
      localStorage.setItem(THEME_KEY, theme);
    } catch {
      // Theme switching still works when browser storage is unavailable.
    }
  }, [theme]);

  useEffect(() => {
    const closeMenu = (event: KeyboardEvent) => {
      if (event.key === "Escape") setMenuOpen(false);
    };
    window.addEventListener("keydown", closeMenu);
    return () => window.removeEventListener("keydown", closeMenu);
  }, []);

  return (
    <>
      <a className="skip-link" href="#main">
        Skip to content
      </a>
      <header className="site-header">
        <div className="container header-inner">
          <a href="#main" aria-label="RSQL home">
            <Logo />
          </a>
          <nav className="desktop-nav" aria-label="Main navigation">
            {navigation.map((item) => (
              <a key={item.href} href={item.href}>
                {item.label}
              </a>
            ))}
          </nav>
          <div className="header-actions">
            <a
              className="icon-button github-link"
              href={REPOSITORY_URL}
              aria-label="RSQL on GitHub"
              target="_blank"
              rel="noreferrer"
            >
              <Github size={19} />
            </a>
            <button
              className="icon-button"
              type="button"
              onClick={() => setTheme(theme === "dark" ? "light" : "dark")}
              aria-label={`Switch to ${theme === "dark" ? "light" : "dark"} theme`}
            >
              {theme === "dark" ? <Sun size={18} /> : <Moon size={18} />}
            </button>
            <a className="button button-small button-primary header-download" href={DOWNLOAD_URL}>
              <Download size={14} /> Download
            </a>
            <button
              className="icon-button menu-button"
              type="button"
              aria-label={menuOpen ? "Close navigation" : "Open navigation"}
              aria-expanded={menuOpen}
              aria-controls="mobile-navigation"
              onClick={() => setMenuOpen(!menuOpen)}
            >
              {menuOpen ? <X size={22} /> : <Menu size={22} />}
            </button>
          </div>
        </div>
        <nav
          id="mobile-navigation"
          className="mobile-nav"
          aria-label="Mobile navigation"
          hidden={!menuOpen}
        >
          {navigation.map((item) => (
            <a key={item.href} href={item.href} onClick={() => setMenuOpen(false)}>
              {item.label}
              <ArrowUpRight size={16} />
            </a>
          ))}
          <a href={DOWNLOAD_URL}>
            Download RSQL
            <Download size={16} />
          </a>
        </nav>
      </header>

      <main id="main">
        <Hero />
        <Features />
        <Playground />
        <Architecture />
        <FAQ />
        <CTA />
      </main>

      <footer className="site-footer container">
        <div className="footer-top">
          <div>
            <a href="#main" aria-label="RSQL home">
              <Logo />
            </a>
            <p>
              A little less friction.
              <br />A lot more SQL.
            </p>
          </div>
          <nav aria-label="Footer navigation">
            <div>
              <span>Product</span>
              <a href="#features">Features</a>
              <a href="#demo">Playground</a>
              <a href={DOWNLOAD_URL}>
                Releases <ArrowUpRight size={12} />
              </a>
            </div>
            <div>
              <span>Open source</span>
              <a href={REPOSITORY_URL}>
                GitHub <ArrowUpRight size={12} />
              </a>
              <a href={DOCS_URL}>
                Documentation <ArrowUpRight size={12} />
              </a>
              <a href={`${REPOSITORY_URL}/issues`}>
                Report an issue <ArrowUpRight size={12} />
              </a>
            </div>
          </nav>
        </div>
        <div className="footer-bottom">
          <span>Made for people who work with data.</span>
          <a href={`${REPOSITORY_URL}/blob/main/LICENSE`}>
            Free software. MIT licensed. <ArrowUpRight size={12} />
          </a>
          <span className="footer-status">
            <span className="status-dot" /> Built with Rust & care
          </span>
        </div>
      </footer>
    </>
  );
}

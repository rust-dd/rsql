import { Copy, Download } from "lucide-react";
import { useEffect, useId, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { toast } from "sonner";
import { copyToClipboard, type ExportFormat, exportResults } from "@/lib/export";
import type { CellValue } from "@/lib/wire";

interface ToolbarExportProps {
  columns: string[];
  filteredRows: CellValue[][];
  hasResult: boolean;
}

export function ToolbarExport({ columns, filteredRows, hasResult }: ToolbarExportProps) {
  const [exportOpen, setExportOpen] = useState(false);
  const exportRef = useRef<HTMLDivElement>(null);
  const menuRef = useRef<HTMLDivElement>(null);
  const menuId = useId();
  const [busy, setBusy] = useState(false);

  const performExport = async (format: ExportFormat, clipboard = false) => {
    if (!hasResult || busy) return;
    setBusy(true);
    setExportOpen(false);
    try {
      if (clipboard) {
        await copyToClipboard(format, columns, filteredRows);
        toast.success("Results copied");
      } else if (await exportResults(format, columns, filteredRows)) {
        toast.success("Results exported");
      }
    } catch (error) {
      toast.error(clipboard ? "Could not copy results" : "Could not export results", {
        description: error instanceof Error ? error.message : String(error),
      });
    } finally {
      setBusy(false);
    }
  };

  useEffect(() => {
    if (!exportOpen) return;
    menuRef.current?.querySelector<HTMLButtonElement>("button")?.focus();
    const close = () => setExportOpen(false);
    window.addEventListener("resize", close);
    return () => window.removeEventListener("resize", close);
  }, [exportOpen]);

  return (
    <div className="relative" ref={exportRef}>
      <button
        type="button"
        onClick={() => setExportOpen(!exportOpen)}
        disabled={busy}
        aria-haspopup="menu"
        aria-expanded={exportOpen}
        aria-controls={menuId}
        className="flex items-center gap-1 px-2 py-0.5 rounded text-xs font-mono text-muted-foreground hover:text-foreground hover:bg-accent transition-colors"
      >
        <Download className="h-3 w-3" />
        {busy ? "Exporting…" : "Export"}
      </button>
      {exportOpen &&
        createPortal(
          <>
            <div
              className="fixed inset-0"
              style={{ zIndex: 9998 }}
              onClick={() => setExportOpen(false)}
            />
            <div
              ref={menuRef}
              id={menuId}
              role="menu"
              aria-label="Export results"
              className="fixed w-52 max-h-[calc(100vh-16px)] overflow-y-auto rounded-md border border-border bg-popover shadow-md py-1"
              onKeyDown={(event) => {
                const items = Array.from(
                  event.currentTarget.querySelectorAll<HTMLButtonElement>("button"),
                );
                const index = items.indexOf(document.activeElement as HTMLButtonElement);
                if (event.key === "Escape") {
                  setExportOpen(false);
                  exportRef.current?.querySelector("button")?.focus();
                } else if (event.key === "ArrowDown" || event.key === "ArrowUp") {
                  event.preventDefault();
                  items[
                    (index + (event.key === "ArrowDown" ? 1 : -1) + items.length) % items.length
                  ]?.focus();
                } else if (event.key === "Tab") setExportOpen(false);
              }}
              style={{
                zIndex: 9999,
                top: (() => {
                  const r = exportRef.current?.getBoundingClientRect();
                  return r ? Math.max(8, Math.min(r.bottom + 4, window.innerHeight - 380)) : 8;
                })(),
                left: (() => {
                  const r = exportRef.current?.getBoundingClientRect();
                  return r ? Math.max(8, Math.min(r.right - 208, window.innerWidth - 216)) : 8;
                })(),
              }}
            >
              <div className="px-2 py-1 text-[10px] font-semibold text-muted-foreground uppercase tracking-wider">
                Download
              </div>
              {(["csv", "json", "sql", "markdown", "xml"] as ExportFormat[]).map((fmt) => (
                <button
                  key={fmt}
                  type="button"
                  role="menuitem"
                  onClick={() => void performExport(fmt)}
                  className="flex w-full items-center gap-2 px-3 py-1.5 text-xs font-mono hover:bg-accent transition-colors"
                >
                  <Download className="h-3 w-3 text-muted-foreground" />
                  {fmt.toUpperCase()}
                </button>
              ))}
              <div className="border-t border-border my-1" />
              <div className="px-2 py-1 text-[10px] font-semibold text-muted-foreground uppercase tracking-wider">
                Copy to clipboard
              </div>
              {(["csv", "json", "sql", "markdown"] as ExportFormat[]).map((fmt) => (
                <button
                  key={`copy-${fmt}`}
                  type="button"
                  role="menuitem"
                  onClick={() => void performExport(fmt, true)}
                  className="flex w-full items-center gap-2 px-3 py-1.5 text-xs font-mono hover:bg-accent transition-colors"
                >
                  <Copy className="h-3 w-3 text-muted-foreground" />
                  {fmt.toUpperCase()}
                </button>
              ))}
            </div>
          </>,
          document.body,
        )}
    </div>
  );
}

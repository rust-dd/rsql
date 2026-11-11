import { Check, Loader2, X } from "lucide-react";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "../ui/dialog";

interface ToolbarEditProps {
  pending: { updates: number; deletes: number };
  sessionMatchesEditor: boolean;
  editError: string | null;
  isCommitting: boolean;
  confirmingApply: boolean;
  onRequestApply: () => void;
  onConfirmApply: () => void;
  onCancelApply: () => void;
  onDiscard: () => void;
}

function summarize({ updates, deletes }: { updates: number; deletes: number }): string {
  const parts: string[] = [];
  if (updates > 0) parts.push(`${updates} update${updates === 1 ? "" : "s"}`);
  if (deletes > 0) parts.push(`${deletes} deletion${deletes === 1 ? "" : "s"}`);
  return parts.join(" and ");
}

export function ToolbarEdit({
  pending,
  sessionMatchesEditor,
  editError,
  isCommitting,
  confirmingApply,
  onRequestApply,
  onConfirmApply,
  onCancelApply,
  onDiscard,
}: ToolbarEditProps) {
  const total = pending.updates + pending.deletes;
  const summary = summarize(pending);

  return (
    <>
      <span
        className="text-xs text-muted-foreground"
        title={
          'Type NULL (any letter case) for SQL NULL, or use double quotes for literal text, e.g. "NULL". An empty cell is an empty string.'
        }
      >
        NULL = SQL NULL · &quot;NULL&quot; = text
      </span>
      {editError && (
        <span className="text-xs text-destructive max-w-[240px] truncate" title={editError}>
          {editError}
        </span>
      )}
      {!sessionMatchesEditor && (
        <span
          className="text-xs text-destructive max-w-[240px] truncate"
          title="The editor no longer targets the table this edit was started on. Discard the edit or restore the original query."
        >
          Query changed — cannot apply
        </span>
      )}
      <button
        type="button"
        onClick={onRequestApply}
        disabled={total === 0 || isCommitting || !sessionMatchesEditor}
        className="flex items-center gap-1 px-2.5 py-1 rounded text-xs font-mono bg-success text-success-foreground hover:bg-success/90 transition-colors disabled:opacity-50"
      >
        {isCommitting ? (
          <Loader2 className="h-3 w-3 animate-spin" />
        ) : (
          <Check className="h-3 w-3" />
        )}
        Apply{total > 0 ? ` (${total})` : ""}
      </button>
      <Dialog
        open={confirmingApply}
        onOpenChange={(open) => {
          if (!open) onCancelApply();
        }}
      >
        <DialogContent>
          <DialogHeader>
            <DialogTitle>Apply changes</DialogTitle>
            <DialogDescription>
              This will apply {summary} in a single transaction. Deletions cannot be undone. If any
              statement does not match exactly one row, nothing is changed.
            </DialogDescription>
          </DialogHeader>
          <DialogFooter>
            <button
              type="button"
              onClick={onCancelApply}
              className="px-3 py-1.5 rounded-lg text-xs font-mono text-muted-foreground hover:text-foreground hover:bg-accent transition-colors"
            >
              Cancel
            </button>
            <button
              type="button"
              onClick={onConfirmApply}
              className={`px-3 py-1.5 rounded-lg text-xs font-mono text-destructive-foreground transition-colors ${
                pending.deletes > 0
                  ? "bg-destructive hover:bg-destructive/90"
                  : "bg-success hover:bg-success/90"
              }`}
            >
              Apply {summary}
            </button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
      <button
        type="button"
        onClick={onDiscard}
        disabled={isCommitting}
        className="flex items-center gap-1 px-2.5 py-1 rounded text-xs font-mono text-muted-foreground hover:text-foreground hover:bg-accent transition-colors disabled:opacity-50"
      >
        <X className="h-3 w-3" />
        Discard
      </button>
    </>
  );
}

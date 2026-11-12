import type { TextSelection } from "@/lib/sql-statements";

type Slot = "main" | "split";

export interface EditorSelection extends TextSelection {
  text: string;
}

const readers = new Map<string, () => EditorSelection | undefined>();

/** The editor showing a tab's pane reports its text and selection when execution asks. */
export function trackEditorSelection(
  tabId: string,
  slot: Slot,
  read: () => EditorSelection | undefined,
): () => void {
  const key = `${tabId}:${slot}`;
  readers.set(key, read);
  return () => {
    if (readers.get(key) === read) readers.delete(key);
  };
}

export function editorSelection(tabId: string, slot: Slot): EditorSelection | undefined {
  return readers.get(`${tabId}:${slot}`)?.();
}

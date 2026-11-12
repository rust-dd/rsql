import type { HistoryFilter } from "@/tauri";

export const TIME_RANGES = {
  any: { label: "Any time", ms: undefined },
  day: { label: "Last 24 hours", ms: 24 * 60 * 60 * 1000 },
  week: { label: "Last 7 days", ms: 7 * 24 * 60 * 60 * 1000 },
  month: { label: "Last 30 days", ms: 30 * 24 * 60 * 60 * 1000 },
} as const;

export type TimeRange = keyof typeof TIME_RANGES;
export type StatusFilter = "all" | "success" | "failure";

export interface HistoryForm {
  text: string;
  projectId: string;
  database: string;
  status: StatusFilter;
  range: TimeRange;
}

export const EMPTY_HISTORY_FORM: HistoryForm = {
  text: "",
  projectId: "",
  database: "",
  status: "all",
  range: "any",
};

export function toHistoryFilter(form: HistoryForm, now: number): HistoryFilter {
  const since = TIME_RANGES[form.range].ms;
  return {
    text: form.text.trim() || undefined,
    projectId: form.projectId || undefined,
    database: form.database || undefined,
    success: form.status === "all" ? undefined : form.status === "success",
    since: since === undefined ? undefined : now - since,
  };
}

export function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}

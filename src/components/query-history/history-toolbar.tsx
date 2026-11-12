import { Search, Trash2 } from "lucide-react";
import { useState } from "react";
import {
  formatBytes,
  type HistoryForm,
  type StatusFilter,
  TIME_RANGES,
  type TimeRange,
} from "@/lib/history-filter";
import type { HistoryStatus } from "@/tauri";
import { Button } from "../ui/button";
import { Input } from "../ui/input";

interface HistoryToolbarProps {
  text: string;
  form: HistoryForm;
  connections: [id: string, name: string][];
  databases: string[];
  status: HistoryStatus | null;
  onText: (text: string) => void;
  onForm: (form: HistoryForm) => void;
  onEnabled: (enabled: boolean) => void;
  onClear: () => void;
}

function FilterSelect({
  label,
  value,
  options,
  onChange,
}: {
  label: string;
  value: string;
  options: [value: string, label: string][];
  onChange: (value: string) => void;
}) {
  return (
    <select
      aria-label={label}
      value={value}
      onChange={(event) => onChange(event.target.value)}
      className="h-7 max-w-40 rounded-md border border-border/50 bg-input px-2 text-xs text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-primary/40"
    >
      {options.map(([optionValue, optionLabel]) => (
        <option key={optionValue} value={optionValue}>
          {optionLabel}
        </option>
      ))}
    </select>
  );
}

export function HistoryToolbar(props: HistoryToolbarProps) {
  const { form, status, onForm } = props;
  const [confirming, setConfirming] = useState(false);

  return (
    <div className="flex flex-shrink-0 flex-col gap-2 border-b border-border px-4 py-2">
      <div className="flex flex-wrap items-center gap-2">
        <div className="relative min-w-48 flex-1">
          <Search className="pointer-events-none absolute left-2 top-1/2 h-3 w-3 -translate-y-1/2 text-muted-foreground" />
          <Input
            value={props.text}
            onChange={(event) => props.onText(event.target.value)}
            placeholder="Search SQL"
            aria-label="Search SQL"
            className="h-7 pl-7 text-xs"
          />
        </div>
        <FilterSelect
          label="Connection"
          value={form.projectId}
          options={[["", "All connections"], ...props.connections]}
          onChange={(projectId) => onForm({ ...form, projectId, database: "" })}
        />
        <FilterSelect
          label="Database"
          value={form.database}
          options={[["", "All databases"], ...props.databases.map((d): [string, string] => [d, d])]}
          onChange={(database) => onForm({ ...form, database })}
        />
        <FilterSelect
          label="Status"
          value={form.status}
          options={[
            ["all", "Any status"],
            ["success", "Succeeded"],
            ["failure", "Failed"],
          ]}
          onChange={(value) => onForm({ ...form, status: value as StatusFilter })}
        />
        <FilterSelect
          label="Time"
          value={form.range}
          options={Object.entries(TIME_RANGES).map(([key, range]) => [key, range.label])}
          onChange={(value) => onForm({ ...form, range: value as TimeRange })}
        />
      </div>
      <div className="flex flex-wrap items-center gap-x-4 gap-y-1 text-[11px] text-muted-foreground">
        {status && (
          <span>
            {status.entries.toLocaleString()} saved on this device ({formatBytes(status.bytes)}).
            Kept for {status.maxAgeDays} days, up to {status.maxEntries.toLocaleString()} queries.
          </span>
        )}
        <label className="flex cursor-pointer items-center gap-1.5">
          <input
            type="checkbox"
            checked={status?.enabled ?? true}
            disabled={!status}
            onChange={(event) => props.onEnabled(event.target.checked)}
          />
          Record history
        </label>
        <div className="ml-auto flex items-center gap-1">
          {confirming ? (
            <>
              <span>Delete all saved history?</span>
              <Button
                variant="ghost"
                size="sm"
                className="h-6 text-xs text-destructive"
                onClick={() => {
                  setConfirming(false);
                  props.onClear();
                }}
              >
                Delete
              </Button>
              <Button
                variant="ghost"
                size="sm"
                className="h-6 text-xs"
                onClick={() => setConfirming(false)}
              >
                Cancel
              </Button>
            </>
          ) : (
            <Button
              variant="ghost"
              size="sm"
              className="h-6 gap-1 text-xs"
              disabled={!status?.entries}
              onClick={() => setConfirming(true)}
            >
              <Trash2 className="h-3 w-3" />
              Clear
            </Button>
          )}
        </div>
      </div>
    </div>
  );
}

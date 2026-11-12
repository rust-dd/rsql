import { beforeEach, describe, expect, it, vi } from "vitest";

const tauri = vi.hoisted(() => ({
  historyRecord: vi.fn(),
  historyStatus: vi.fn(),
  historySetEnabled: vi.fn(),
  historyClear: vi.fn(),
}));
vi.mock("@/tauri", () => tauri);

import { useHistoryStore } from "./history-store";

const status = {
  enabled: true,
  entries: 1,
  bytes: 8,
  maxEntries: 10_000,
  maxAgeDays: 90,
  maxBytes: 32 * 1024 * 1024,
};

const entry = {
  projectId: "p1",
  database: "sales",
  sql: "SELECT 1",
  executionTime: 4,
  rowCount: 1,
  success: false,
  error: "boom",
  timestamp: 1_800_000_000_000,
};

beforeEach(() => {
  vi.clearAllMocks();
  tauri.historyStatus.mockResolvedValue(status);
  useHistoryStore.setState({ entries: [], status: null, revision: 0 });
});

describe("history store", () => {
  it("saves an execution in the background and reloads the saved history", async () => {
    let save: (saved: boolean) => void = () => {};
    tauri.historyRecord.mockReturnValue(new Promise((resolve) => (save = resolve)));

    useHistoryStore.getState().addEntry(entry);

    expect(useHistoryStore.getState().entries).toHaveLength(1);
    expect(tauri.historyRecord).toHaveBeenCalledWith({
      projectId: "p1",
      database: "sales",
      sql: "SELECT 1",
      executedAt: entry.timestamp,
      durationMs: 4,
      rowCount: 1,
      success: false,
      error: "boom",
    });
    expect(useHistoryStore.getState().revision).toBe(0);

    save(true);
    await vi.waitFor(() => expect(useHistoryStore.getState().status).toEqual(status));
    expect(useHistoryStore.getState().revision).toBe(1);
  });

  it("does not reload when recording is off or saving fails", async () => {
    const logged = vi.spyOn(console, "error").mockImplementation(() => {});
    tauri.historyRecord.mockResolvedValueOnce(false).mockRejectedValueOnce(new Error("locked"));

    useHistoryStore.getState().addEntry(entry);
    useHistoryStore.getState().addEntry(entry);

    await vi.waitFor(() => expect(logged).toHaveBeenCalledOnce());
    expect(useHistoryStore.getState().revision).toBe(0);
    expect(tauri.historyStatus).not.toHaveBeenCalled();
    logged.mockRestore();
  });

  it("clears saved and session history", async () => {
    tauri.historyRecord.mockResolvedValue(false);
    useHistoryStore.getState().addEntry(entry);

    await useHistoryStore.getState().clearHistory();

    expect(tauri.historyClear).toHaveBeenCalledOnce();
    expect(useHistoryStore.getState().entries).toEqual([]);
    expect(useHistoryStore.getState().revision).toBe(1);
    expect(useHistoryStore.getState().status).toEqual(status);
  });

  it("refreshes the status after switching recording", async () => {
    tauri.historyStatus.mockResolvedValue({ ...status, enabled: false });

    await useHistoryStore.getState().setEnabled(false);

    expect(tauri.historySetEnabled).toHaveBeenCalledWith(false);
    expect(useHistoryStore.getState().status?.enabled).toBe(false);
  });
});

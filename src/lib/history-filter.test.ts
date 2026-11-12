import { describe, expect, it } from "vitest";
import { EMPTY_HISTORY_FORM, formatBytes, toHistoryFilter } from "./history-filter";

const NOW = 1_800_000_000_000;

describe("toHistoryFilter", () => {
  it("leaves every filter out for an empty form", () => {
    expect(toHistoryFilter(EMPTY_HISTORY_FORM, NOW)).toEqual({
      text: undefined,
      projectId: undefined,
      database: undefined,
      success: undefined,
      since: undefined,
    });
  });

  it("maps text, connection, database, status and time range", () => {
    const filter = toHistoryFilter(
      { text: "  orders ", projectId: "p1", database: "sales", status: "failure", range: "week" },
      NOW,
    );
    expect(filter).toEqual({
      text: "orders",
      projectId: "p1",
      database: "sales",
      success: false,
      since: NOW - 7 * 24 * 60 * 60 * 1000,
    });
    expect(toHistoryFilter({ ...EMPTY_HISTORY_FORM, status: "success" }, NOW).success).toBe(true);
  });

  it("ignores whitespace-only search text", () => {
    expect(toHistoryFilter({ ...EMPTY_HISTORY_FORM, text: "   " }, NOW).text).toBeUndefined();
  });
});

describe("formatBytes", () => {
  it("picks a readable unit", () => {
    expect(formatBytes(512)).toBe("512 B");
    expect(formatBytes(1536)).toBe("1.5 KB");
    expect(formatBytes(32 * 1024 * 1024)).toBe("32.0 MB");
  });
});

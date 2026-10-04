import { expect, it, vi } from "vitest";
import { streamVirtualCSV } from "./stream-export";
import { encodeRows } from "./wire";

it("streams multiple pages faithfully while handling partial file writes", async () => {
  const pages = [encodeRows([[null], [""]]), encodeRows([['a\r\n"b']])];
  const fetchPage = vi.fn().mockImplementation(async (offset) => pages[offset / 2]);
  const onProgress = vi.fn();
  const chunks: Uint8Array[] = [];
  await streamVirtualCSV({
    columns: ["value"],
    totalRows: 3,
    pageSize: 2,
    fetchPage,
    signal: new AbortController().signal,
    onProgress,
    write: async (bytes) => {
      const size = Math.min(3, bytes.length);
      chunks.push(bytes.slice(0, size));
      return size;
    },
  });
  expect(new TextDecoder().decode(Uint8Array.from(chunks.flatMap((bytes) => [...bytes])))).toBe(
    'value\n\n""\n"a\r\n""b"\n',
  );
  expect(fetchPage.mock.calls).toEqual([
    [0, 2],
    [2, 2],
  ]);
  expect(onProgress.mock.calls).toEqual([[2], [3]]);
});

it("stops fetching and writing when cancelled between pages", async () => {
  const controller = new AbortController();
  const fetchPage = vi.fn().mockResolvedValue(encodeRows([["one"]]));
  await expect(
    streamVirtualCSV({
      columns: ["value"],
      totalRows: 2,
      pageSize: 1,
      fetchPage,
      signal: controller.signal,
      write: async (bytes) => bytes.length,
      onProgress: () => controller.abort(),
    }),
  ).rejects.toThrow();
  expect(fetchPage).toHaveBeenCalledOnce();
});

it("rejects expired or short pages instead of reporting a complete export", async () => {
  const progress = vi.fn();
  await expect(
    streamVirtualCSV({
      columns: ["value"],
      totalRows: 2,
      pageSize: 2,
      fetchPage: async () => encodeRows([["one"]]),
      signal: new AbortController().signal,
      write: async (bytes) => bytes.length,
      onProgress: progress,
    }),
  ).rejects.toThrow("incomplete");
  expect(progress).not.toHaveBeenCalled();
});

it("propagates disk failures", async () => {
  await expect(
    streamVirtualCSV({
      columns: ["value"],
      totalRows: 1,
      pageSize: 1,
      fetchPage: vi.fn(),
      signal: new AbortController().signal,
      write: async () => {
        throw new Error("disk full");
      },
      onProgress: vi.fn(),
    }),
  ).rejects.toThrow("disk full");
});

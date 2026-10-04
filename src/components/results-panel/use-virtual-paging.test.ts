import { createElement } from "react";
import { renderToString } from "react-dom/server";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import * as cache from "@/lib/virtual-cache";
import { decodePage } from "@/lib/wire";
import { useVirtualPaging } from "./use-virtual-paging";

const { fetchPage, selectedQuery } = vi.hoisted(() => ({
  fetchPage: vi.fn(),
  selectedQuery: { id: "paging-test" },
}));

vi.mock("@/lib/database-driver", () => ({
  DriverFactory: { getDriver: () => ({ fetchPage }) },
}));

vi.mock("@/stores/project-store", () => ({
  useProjectStore: { getState: () => ({ projects: { project: { driver: "PGSQL" } } }) },
}));

vi.mock("@/stores/tab-store", () => ({
  useTabStore: {
    getState: () => ({
      selectedTabIndex: 0,
      tabs: [{ virtualQuery: { queryId: selectedQuery.id } }],
    }),
  },
}));

function pagingConsumer() {
  let paging: ReturnType<typeof useVirtualPaging> | undefined;
  function Consumer() {
    paging = useVirtualPaging({
      projectId: "project",
      vq: { queryId: "paging-test", totalRows: 5, pageSize: 2, colCount: 1, time: 0 },
    });
    return null;
  }
  renderToString(createElement(Consumer));
  if (!paging) throw new Error("Paging consumer did not render");
  return paging;
}

beforeEach(() => {
  fetchPage.mockReset();
  selectedQuery.id = "paging-test";
});

afterEach(() => cache.clearQuery("paging-test"));

describe("virtual page fetching", () => {
  it("preserves values across the initial page and two fetched pages", async () => {
    cache.setPage("paging-test", 0, decodePage("\x1dN\x1enull"));
    fetchPage
      .mockResolvedValueOnce("\x1dE\x1eárvíz 🦀\x1dC\x1dB\x1dA")
      .mockResolvedValueOnce("\x1dE");
    const paging = pagingConsumer();

    paging.handlePageNeeded(1);
    paging.handlePageNeeded(2);

    await vi.waitFor(() => {
      expect(cache.getRow("paging-test", 2, 2)).toEqual([""]);
      expect(cache.getRow("paging-test", 3, 2)).toEqual(["árvíz 🦀\x1d\x1e\x1f"]);
      expect(cache.getRow("paging-test", 4, 2)).toEqual([""]);
    });
    expect(cache.getRow("paging-test", 0, 2)).toEqual([null]);
    expect(cache.getRow("paging-test", 1, 2)).toEqual(["null"]);
    expect(fetchPage).toHaveBeenCalledWith("project", "paging-test", 1, 2, 2);
    expect(fetchPage).toHaveBeenCalledWith("project", "paging-test", 1, 4, 2);
  });

  it("decodes NULL on a fetched page", async () => {
    fetchPage.mockResolvedValueOnce("\x1dN\x1enull");
    pagingConsumer().handlePageNeeded(1);
    await vi.waitFor(() => {
      expect(cache.getRow("paging-test", 2, 2)).toEqual([null]);
      expect(cache.getRow("paging-test", 3, 2)).toEqual(["null"]);
    });
  });

  it("discards a response for a query that is no longer selected", async () => {
    fetchPage.mockResolvedValueOnce("old\x1eresult");
    pagingConsumer().handlePageNeeded(1);
    selectedQuery.id = "another-query";
    await vi.waitFor(() => expect(fetchPage).toHaveResolved());
    expect(cache.hasPage("paging-test", 1)).toBe(false);
  });
});

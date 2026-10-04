import { useCallback, useEffect, useRef } from "react";
import { DriverFactory } from "@/lib/database-driver";
import * as virtualCache from "@/lib/virtual-cache";
import { decodePage } from "@/lib/wire";
import { useProjectStore } from "@/stores/project-store";
import { useTabStore } from "@/stores/tab-store";
import {
  CACHE_WINDOW_PAGES,
  MAX_CONCURRENT_PAGE_FETCHES,
  MAX_QUEUED_PAGE_FETCHES,
} from "./constants";

interface VirtualQuery {
  queryId: string;
  totalRows: number;
  time: number;
  pageSize: number;
  colCount: number;
}

interface UseVirtualPagingArgs {
  vq: VirtualQuery | undefined;
  projectId: string | undefined;
}

export function useVirtualPaging({ vq, projectId }: UseVirtualPagingArgs) {
  const loadingPages = useRef(new Set<number>());
  const queuedPages = useRef<number[]>([]);
  const queuedPageSet = useRef(new Set<number>());
  const activeFetches = useRef(0);
  const latestRequestedPage = useRef(0);
  const gridRef = useRef<{ invalidatePage: (pageIndex: number) => void }>(null);
  const virtualViewportRows = useRef(new Map<string, number>());

  useEffect(() => {
    loadingPages.current.clear();
    queuedPages.current = [];
    queuedPageSet.current.clear();
    activeFetches.current = 0;
  }, []);

  const handleViewportRowChange = useCallback(
    (rowIndex: number) => {
      if (!vq?.queryId) return;
      virtualViewportRows.current.set(vq.queryId, rowIndex);
    },
    [vq?.queryId],
  );

  const restoreRowIndex = vq?.queryId ? (virtualViewportRows.current.get(vq.queryId) ?? 0) : 0;

  const fetchPage = useCallback(
    async (pageIndex: number) => {
      if (!vq || !projectId) return;
      const d = useProjectStore.getState().projects[projectId];
      if (!d) return;
      const driver = DriverFactory.getDriver(d.driver);
      if (!driver.fetchPage) return;

      const offset = pageIndex * vq.pageSize;
      const packed = await driver.fetchPage(
        projectId,
        vq.queryId,
        vq.colCount,
        offset,
        vq.pageSize,
      );

      // Drop stale page responses after tab/query switches.
      const selectedIdx = useTabStore.getState().selectedTabIndex;
      const selectedTab = useTabStore.getState().tabs[selectedIdx];
      if (selectedTab?.virtualQuery?.queryId !== vq.queryId) return;

      const rows = decodePage(packed);
      const expectedRows = Math.max(0, Math.min(vq.pageSize, vq.totalRows - offset));
      if (expectedRows > 0 && rows.length === 0) {
        // Keep page as "missing" so viewport observer can retry instead of caching a permanent empty page.
        return;
      }
      virtualCache.setPage(vq.queryId, pageIndex, rows);
      // Evict around the user's latest viewport, not the page that happened to resolve last.
      virtualCache.evictDistant(vq.queryId, latestRequestedPage.current, CACHE_WINDOW_PAGES);
      gridRef.current?.invalidatePage(pageIndex);
    },
    [vq, projectId],
  );

  const pumpQueue = useCallback(() => {
    if (!vq || !projectId) return;

    if (queuedPages.current.length > 1) {
      const target = latestRequestedPage.current;
      queuedPages.current.sort((a, b) => Math.abs(a - target) - Math.abs(b - target));
    }

    while (activeFetches.current < MAX_CONCURRENT_PAGE_FETCHES && queuedPages.current.length > 0) {
      const pageIndex = queuedPages.current.shift()!;
      queuedPageSet.current.delete(pageIndex);

      if (loadingPages.current.has(pageIndex) || virtualCache.hasPage(vq.queryId, pageIndex)) {
        continue;
      }

      loadingPages.current.add(pageIndex);
      activeFetches.current += 1;

      void fetchPage(pageIndex).finally(() => {
        loadingPages.current.delete(pageIndex);
        activeFetches.current -= 1;
        pumpQueue();
      });
    }
  }, [vq, projectId, fetchPage]);

  const handlePageNeeded = useCallback(
    (pageIndex: number) => {
      if (!vq || !projectId) return;
      latestRequestedPage.current = pageIndex;
      if (
        loadingPages.current.has(pageIndex) ||
        virtualCache.hasPage(vq.queryId, pageIndex) ||
        queuedPageSet.current.has(pageIndex)
      ) {
        return;
      }

      if (queuedPages.current.length >= MAX_QUEUED_PAGE_FETCHES) {
        queuedPages.current = queuedPages.current.filter((p) => Math.abs(p - pageIndex) <= 8);
        queuedPageSet.current = new Set(queuedPages.current);
      }

      queuedPages.current.push(pageIndex);
      queuedPageSet.current.add(pageIndex);
      pumpQueue();
    },
    [vq, projectId, pumpQueue],
  );

  useEffect(() => {
    if (!vq) return;
    const anchorPage = Math.max(0, Math.floor(restoreRowIndex / vq.pageSize));
    const startPage = Math.max(0, anchorPage - 1);
    const endPage = Math.min(anchorPage + 3, Math.ceil(vq.totalRows / vq.pageSize) - 1);
    for (let p = startPage; p <= endPage; p++) {
      handlePageNeeded(p);
    }
  }, [vq?.queryId, vq?.totalRows, vq?.pageSize, restoreRowIndex, handlePageNeeded, vq]);

  return {
    gridRef,
    handlePageNeeded,
    handleViewportRowChange,
    restoreRowIndex,
  };
}

import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { DriverFactory } from "@/lib/database-driver";
import { createPageLoader } from "@/lib/page-loader";
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
  const gridRef = useRef<{ invalidatePage: (pageIndex: number) => void }>(null);
  const [failure, setFailure] = useState<{ queryId: string; message: string } | null>(null);
  const queryId = vq?.queryId;
  const pageSize = vq?.pageSize ?? 1;
  const totalRows = vq?.totalRows ?? 0;
  const colCount = vq?.colCount ?? 0;

  const loader = useMemo(() => {
    const isCurrent = () => {
      const state = useTabStore.getState();
      const tab = state.tabs[state.selectedTabIndex];
      return tab?.virtualQuery?.queryId === queryId;
    };
    const controller = createPageLoader({
      isCurrent,
      hasPage: (page) => !!queryId && virtualCache.hasPage(queryId, page),
      maxConcurrent: MAX_CONCURRENT_PAGE_FETCHES,
      maxQueued: MAX_QUEUED_PAGE_FETCHES,
      pageCount: Math.ceil(totalRows / pageSize),
      onError: (message) => setFailure(message && queryId ? { queryId, message } : null),
      load: async (page, isCurrent) => {
        if (!queryId || !projectId) return;
        const project = useProjectStore.getState().projects[projectId];
        if (!project) throw new Error("Connection is no longer available.");
        const driver = DriverFactory.getDriver(project.driver);
        if (!driver.fetchPage) throw new Error("This connection cannot load result pages.");
        const packed = await driver.fetchPage(
          projectId,
          queryId,
          colCount,
          page * pageSize,
          pageSize,
        );
        if (!isCurrent()) return;
        const rows = decodePage(packed);
        const expected = Math.min(pageSize, totalRows - page * pageSize);
        if (rows.length !== expected)
          throw new Error("Result rows are no longer available. Run the query again.");
        virtualCache.setPage(queryId, page, rows);
        virtualCache.evictDistant(queryId, controller.target, CACHE_WINDOW_PAGES);
        gridRef.current?.invalidatePage(page);
      },
    });
    return controller;
  }, [queryId, projectId, pageSize, totalRows, colCount]);

  useEffect(() => {
    loader.resume();
    return () => loader.dispose();
  }, [loader]);

  const handleViewportRowChange = useCallback(
    (row: number) => {
      if (queryId) virtualCache.setViewportRow(queryId, row);
    },
    [queryId],
  );
  const restoreRowIndex = queryId ? virtualCache.getViewportRow(queryId) : 0;
  const handlePageNeeded = useCallback((page: number) => loader.request(page), [loader]);

  useEffect(() => {
    if (!queryId) return;
    const anchor = Math.max(0, Math.floor(restoreRowIndex / pageSize));
    for (let page = Math.max(0, anchor - 1); page <= anchor + 3; page++) loader.request(page);
  }, [loader, queryId, pageSize, restoreRowIndex]);

  return {
    gridRef,
    handlePageNeeded,
    handleViewportRowChange,
    restoreRowIndex,
    pageError: failure?.queryId === queryId ? failure?.message : undefined,
    retryPages: () => loader.retry(),
  };
}

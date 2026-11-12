import type { Tab } from "@/types";

export interface PersistedTabs {
  tabs: Tab[];
  selectedTabIndex: number;
  selectedTabId?: string;
}

const isSaved = (tab: Tab | undefined): tab is Tab =>
  tab != null && tab.type !== "terminal" && tab.type !== "notify";

const toStored = (tab: Tab): Tab => ({
  id: tab.id,
  type: tab.type,
  projectId: tab.projectId,
  schema: tab.schema,
  title: tab.title,
  editorValue: tab.editorValue,
  isExecuting: false,
  queryTimeout: tab.queryTimeout,
  isSplit: tab.isSplit,
  splitEditorValue: tab.splitEditorValue,
});

/** An unsaved active tab passes the selection to the nearest saved tab before it, else after it. */
function selectedSavedTab(tabs: Tab[], selectedTabIndex: number): Tab | undefined {
  if (tabs.length === 0) return undefined;
  const index = Math.min(Math.max(selectedTabIndex, 0), tabs.length - 1);
  if (isSaved(tabs[index])) return tabs[index];
  return tabs.slice(0, index).reverse().find(isSaved) ?? tabs.slice(index + 1).find(isSaved);
}

export function persistTabs(tabs: Tab[], selectedTabIndex: number): PersistedTabs {
  const saved = tabs.filter(isSaved).map(toStored);
  const selected = selectedSavedTab(tabs, selectedTabIndex);
  return {
    tabs: saved,
    selectedTabIndex: selected ? saved.findIndex((tab) => tab.id === selected.id) : -1,
    selectedTabId: selected?.id,
  };
}

const isStoredTab = (value: unknown): value is Tab =>
  value != null &&
  typeof value === "object" &&
  "id" in value &&
  "type" in value &&
  "title" in value;

/** Selects by stored id; state saved before ids were stored falls back to its index. */
export function restoreTabs(
  persisted: unknown,
): Pick<PersistedTabs, "tabs" | "selectedTabIndex"> | undefined {
  const stored = persisted as Partial<Record<keyof PersistedTabs, unknown>> | undefined;
  if (!Array.isArray(stored?.tabs)) return undefined;
  const tabs = stored.tabs.filter(isStoredTab);
  if (tabs.length === 0) return { tabs: [], selectedTabIndex: -1 };
  const byId = tabs.findIndex((tab) => tab.id === stored.selectedTabId);
  if (byId !== -1) return { tabs, selectedTabIndex: byId };
  const index = Number.isInteger(stored.selectedTabIndex) ? (stored.selectedTabIndex as number) : 0;
  return { tabs, selectedTabIndex: Math.min(Math.max(index, 0), tabs.length - 1) };
}

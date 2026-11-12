export type Slot = "main" | "split";
type Cleanup = () => unknown;

const executions = new Map<string, { cancel: () => void }>();
const results = new Map<string, Cleanup>();
const editors = new Map<string, Cleanup>();

function safely(cleanup: Cleanup) {
  try {
    Promise.resolve(cleanup()).catch((error) => console.error("Resource cleanup failed:", error));
  } catch (error) {
    console.error("Resource cleanup failed:", error);
  }
}

export function trackExecution(tabId: string, slot: Slot, cancel: Cleanup) {
  const key = `${tabId}:${slot}`;
  executions.get(key)?.cancel();
  let active = true;
  const execution = {
    get active() {
      return active;
    },
    cancel() {
      if (!active) return;
      active = false;
      safely(cancel);
      if (executions.get(key) === execution) executions.delete(key);
    },
    finish() {
      active = false;
      if (executions.get(key) === execution) executions.delete(key);
    },
  };
  executions.set(key, execution);
  return execution;
}

export function cancelExecution(tabId: string, slot: Slot) {
  executions.get(`${tabId}:${slot}`)?.cancel();
}

export function releaseResult(tabId: string, slot: Slot) {
  const key = `${tabId}:${slot}`;
  const cleanup = results.get(key);
  results.delete(key);
  if (cleanup) safely(cleanup);
}

export function trackResult(tabId: string, slot: Slot, cleanup: Cleanup) {
  releaseResult(tabId, slot);
  results.set(`${tabId}:${slot}`, cleanup);
}

export function trackTabEditor(tabId: string, slot: Slot, cleanup: Cleanup) {
  editors.set(`${tabId}:${slot}`, cleanup);
}

export function releaseTabEditors(tabId: string) {
  for (const slot of ["main", "split"]) {
    const key = `${tabId}:${slot}`;
    const cleanup = editors.get(key);
    editors.delete(key);
    if (cleanup) safely(cleanup);
  }
}

export function releaseTabResources(tabId: string) {
  cancelExecution(tabId, "main");
  cancelExecution(tabId, "split");
  releaseResult(tabId, "main");
  releaseResult(tabId, "split");
}

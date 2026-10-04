type Slot = "main" | "split";
type Cleanup = () => unknown;

const executions = new Map<string, { cancel: () => void }>();
const results = new Map<string, Cleanup>();

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

export function releaseResult(tabId: string) {
  const cleanup = results.get(tabId);
  results.delete(tabId);
  if (cleanup) safely(cleanup);
}

export function trackResult(tabId: string, cleanup: Cleanup) {
  releaseResult(tabId);
  results.set(tabId, cleanup);
}

export function releaseTabResources(tabId: string) {
  cancelExecution(tabId, "main");
  cancelExecution(tabId, "split");
  releaseResult(tabId);
}

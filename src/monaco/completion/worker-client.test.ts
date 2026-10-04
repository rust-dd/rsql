import { afterEach, expect, it, vi } from "vitest";
import type { ParseRequest, ParseResponse } from "./parse-types";
import { CompletionWorkerClient } from "./worker-client";

const request: ParseRequest = {
  document: "query",
  version: 1,
  sql: "SELECT u. FROM users u",
  caret: { lineNumber: 1, column: 10 },
  wordRange: { startLineNumber: 1, endLineNumber: 1, startColumn: 10, endColumn: 10 },
};

function fixture() {
  const worker = {
    onmessage: null as ((event: MessageEvent<ParseResponse>) => void) | null,
    onerror: null as ((event: ErrorEvent) => void) | null,
    postMessage: vi.fn(),
    terminate: vi.fn(),
  };
  const client = new CompletionWorkerClient(() => worker);
  const respond = (id: number) =>
    worker.onmessage?.({ data: { id, result: null } } as MessageEvent<ParseResponse>);
  return { client, worker, respond };
}

afterEach(() => vi.useRealTimers());

it("coalesces obsolete documents and never lets old replies finish a newer request", async () => {
  const { client, worker, respond } = fixture();
  const first = client.parse(request);
  const second = client.parse({ ...request, version: 2 });
  const third = client.parse({ ...request, version: 3 });
  await expect(first).resolves.toBeNull();
  await expect(second).resolves.toBeNull();
  expect(worker.postMessage).toHaveBeenCalledTimes(1);
  respond(1);
  expect(worker.postMessage).toHaveBeenLastCalledWith({ ...request, version: 3, id: 3 });
  const done = vi.fn();
  third.then(done);
  respond(1);
  await Promise.resolve();
  expect(done).not.toHaveBeenCalled();
  respond(3);
  await expect(third).resolves.toBeNull();
  client.dispose();
});

it("cancels queued work without sending it to the worker", async () => {
  const { client, worker, respond } = fixture();
  const first = client.parse(request);
  let cancel = () => {};
  const dispose = vi.fn();
  const second = client.parse(
    { ...request, document: "other" },
    {
      isCancellationRequested: false,
      onCancellationRequested: (listener) => {
        cancel = listener;
        return { dispose };
      },
    },
  );
  cancel();
  await expect(second).resolves.toBeNull();
  respond(1);
  await first;
  expect(worker.postMessage).toHaveBeenCalledTimes(1);
  expect(dispose).toHaveBeenCalled();
  client.dispose();
});

it("terminates stalled work, rejects waiters, and recovers for the next request", async () => {
  vi.useFakeTimers();
  const { client, worker, respond } = fixture();
  const first = client.parse(request);
  const rejected = expect(first).rejects.toThrow("timed out");
  vi.advanceTimersByTime(8_000);
  await rejected;
  expect(worker.terminate).toHaveBeenCalledOnce();
  const next = client.parse(request);
  respond(2);
  await expect(next).resolves.toBeNull();
  client.dispose();
});

it("settles outstanding work and releases the worker on disposal", async () => {
  const { client, worker } = fixture();
  const first = client.parse(request);
  const second = client.parse({ ...request, document: "another" });
  client.dispose();
  await expect(first).resolves.toBeNull();
  await expect(second).resolves.toBeNull();
  expect(worker.terminate).toHaveBeenCalledOnce();
});

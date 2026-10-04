import type { ParsedCompletion, ParseRequest, ParseResponse } from "./parse-types";

interface ParseWorker {
  onmessage: ((event: MessageEvent<ParseResponse>) => void) | null;
  onerror: ((event: ErrorEvent) => void) | null;
  postMessage(message: ParseRequest & { id: number }): void;
  terminate(): void;
}

interface Cancellation {
  isCancellationRequested: boolean;
  onCancellationRequested(listener: () => void): { dispose(): void };
}

interface Pending {
  id: number;
  request: ParseRequest;
  resolve(result: ParsedCompletion | null): void;
  reject(error: Error): void;
  subscription?: { dispose(): void };
}

export class CompletionWorkerClient {
  private worker: ParseWorker | null = null;
  private sequence = 0;
  private active: Pending | null = null;
  private queue = new Map<string, Pending>();
  private timer: ReturnType<typeof setTimeout> | null = null;

  constructor(
    private createWorker: () => ParseWorker = () =>
      new Worker(new URL("./parse.worker.ts", import.meta.url), { type: "module" }),
  ) {}

  parse(request: ParseRequest, token?: Cancellation): Promise<ParsedCompletion | null> {
    if (token?.isCancellationRequested) return Promise.resolve(null);
    if (this.active?.request.document === request.document) this.finish(this.active, null);
    const previous = this.queue.get(request.document);
    if (previous) this.finish(previous, null);
    return new Promise((resolve, reject) => {
      const pending: Pending = { id: ++this.sequence, request, resolve, reject };
      pending.subscription = token?.onCancellationRequested(() => {
        if (this.queue.get(request.document) === pending) this.queue.delete(request.document);
        this.finish(pending, null);
      });
      this.queue.set(request.document, pending);
      if (this.queue.size > 16) {
        const oldest = this.queue.values().next().value as Pending;
        this.queue.delete(oldest.request.document);
        this.finish(oldest, null);
      }
      this.drain();
    });
  }

  private finish(pending: Pending, result: ParsedCompletion | null, error?: Error) {
    pending.subscription?.dispose();
    if (error) pending.reject(error);
    else pending.resolve(result);
  }

  private drain() {
    if (this.active || !this.queue.size) return;
    try {
      if (!this.worker) {
        this.worker = this.createWorker();
        this.worker.onmessage = ({ data }) => {
          if (data.id !== this.active?.id) return;
          if (this.timer) clearTimeout(this.timer);
          this.finish(
            this.active,
            data.result ?? null,
            data.error ? new Error(data.error) : undefined,
          );
          this.active = null;
          this.drain();
        };
        this.worker.onerror = () => this.reset(new Error("SQL completion worker failed"));
      }
      this.active = this.queue.values().next().value as Pending;
      this.queue.delete(this.active.request.document);
      this.timer = setTimeout(
        () => this.reset(new Error("SQL completion parsing timed out")),
        8_000,
      );
      this.worker.postMessage({ ...this.active.request, id: this.active.id });
    } catch (error) {
      this.reset(error instanceof Error ? error : new Error(String(error)));
    }
  }

  private reset(error?: Error) {
    if (this.timer) clearTimeout(this.timer);
    this.worker?.terminate();
    this.worker = null;
    if (this.active) this.finish(this.active, null, error);
    for (const pending of this.queue.values()) this.finish(pending, null, error);
    this.active = null;
    this.queue.clear();
  }

  dispose() {
    this.reset();
  }
}

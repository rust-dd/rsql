import { describe, expect, it, vi } from "vitest";
import { createPageLoader } from "./page-loader";

function deferred() {
  let resolve = () => {};
  const promise = new Promise<void>((done) => {
    resolve = done;
  });
  return { promise, resolve };
}

function loader(load: (page: number, isCurrent: () => boolean) => Promise<void>) {
  const onError = vi.fn();
  const value = createPageLoader({
    load,
    hasPage: () => false,
    isCurrent: () => true,
    onError,
    maxConcurrent: 1,
    maxQueued: 2,
    pageCount: 100,
  });
  return { value, onError };
}

describe("page request ownership", () => {
  it("settled obsolete work cannot publish or restart its queue", async () => {
    const gate = deferred();
    const published = vi.fn();
    const load = vi.fn(async (_page: number, isCurrent: () => boolean) => {
      await gate.promise;
      if (isCurrent()) published();
    });
    const { value } = loader(load);
    value.request(0);
    value.request(1);
    value.dispose();
    value.resume();
    gate.resolve();
    await vi.waitFor(() => expect(load).toHaveResolved());
    expect(published).not.toHaveBeenCalled();
    expect(load).toHaveBeenCalledTimes(1);
    value.request(2);
    await vi.waitFor(() => expect(published).toHaveBeenCalledTimes(1));
  });

  it("limits automatic retries and exposes an explicit retry", async () => {
    const load = vi.fn().mockRejectedValue(new Error("expired"));
    const { value, onError } = loader(load);
    value.request(0);
    await vi.waitFor(() => expect(onError).toHaveBeenCalledWith("expired"));
    value.request(0);
    expect(load).toHaveBeenCalledTimes(2);
    load.mockResolvedValue(undefined);
    value.retry();
    await vi.waitFor(() => expect(load).toHaveBeenCalledTimes(3));
    expect(onError).toHaveBeenCalledWith(null);
  });

  it("bounds pending work and prioritizes the latest viewport", async () => {
    const gate = deferred();
    const load = vi.fn().mockReturnValueOnce(gate.promise).mockResolvedValue(undefined);
    const { value } = loader(load);
    value.request(0);
    for (let page = 1; page < 10; page++) value.request(page);
    value.request(-1);
    value.request(100);
    gate.resolve();
    await vi.waitFor(() => expect(load).toHaveBeenCalledTimes(3));
    expect(load.mock.calls.map(([page]) => page)).toEqual([0, 9, 8]);
  });
});

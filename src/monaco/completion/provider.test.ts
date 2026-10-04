import { describe, expect, it, vi } from "vitest";
import { registerCompletion } from "./provider";

const { parse } = vi.hoisted(() => ({ parse: vi.fn() }));
vi.mock("./worker-client", () => ({
  CompletionWorkerClient: class {
    parse = parse;
    dispose() {}
  },
}));

/**
 * Just enough Monaco to observe registration. The provider's suggestions are
 * covered by build and pipeline tests; what matters here is that hot reload
 * cannot stack providers, which is what made every suggestion appear twice.
 */
function fakeMonaco() {
  const disposed: number[] = [];
  let next = 0;
  const registered: unknown[] = [];
  return {
    disposed,
    registered,
    monaco: {
      languages: {
        registerCompletionItemProvider: (_language: string, provider: unknown) => {
          const id = next++;
          registered.push(provider);
          return { dispose: () => disposed.push(id) };
        },
        CompletionItemKind: {
          Field: 3,
          Class: 6,
          Function: 1,
          Module: 8,
          Keyword: 17,
          Snippet: 27,
        },
        CompletionItemInsertTextRule: { InsertAsSnippet: 4 },
      },
    } as never,
  };
}

describe("registerCompletion", () => {
  it("discards a response after the model changes", async () => {
    const { monaco, registered } = fakeMonaco();
    registerCompletion(monaco);
    const provider = registered[0] as {
      provideCompletionItems(...args: unknown[]): Promise<{ suggestions: unknown[] }>;
    };
    let version = 1;
    let resolve!: (result: null) => void;
    parse.mockReturnValueOnce(
      new Promise((done) => {
        resolve = done;
      }),
    );
    const model = {
      getValue: () => "SELECT id FROM users",
      getWordUntilPosition: () => ({ startColumn: 8, endColumn: 10 }),
      getVersionId: () => version,
      isDisposed: () => false,
      uri: { toString: () => "query" },
    };
    const result = provider.provideCompletionItems(
      model,
      { lineNumber: 1, column: 10 },
      {},
      { isCancellationRequested: false },
    );
    version++;
    resolve(null);
    expect((await result).suggestions).toEqual([]);
  });

  it("discards an earlier caret request even if the document version is unchanged", async () => {
    const { monaco, registered } = fakeMonaco();
    registerCompletion(monaco);
    const provider = registered[0] as {
      provideCompletionItems(...args: unknown[]): Promise<{ suggestions: unknown[] }>;
    };
    let resolve!: (result: null) => void;
    parse
      .mockReturnValueOnce(
        new Promise((done) => {
          resolve = done;
        }),
      )
      .mockResolvedValueOnce(null);
    const model = {
      getValue: () => "SELECT id FROM users",
      getWordUntilPosition: () => ({ startColumn: 8, endColumn: 10 }),
      getVersionId: () => 1,
      isDisposed: () => false,
      uri: { toString: () => "query" },
    };
    const first = provider.provideCompletionItems(
      model,
      { lineNumber: 1, column: 10 },
      {},
      { isCancellationRequested: false },
    );
    const second = await provider.provideCompletionItems(
      model,
      { lineNumber: 1, column: 11 },
      {},
      { isCancellationRequested: false },
    );
    expect(second.suggestions.length).toBeGreaterThan(0);
    resolve(null);
    expect((await first).suggestions).toEqual([]);
  });
  it("registers one provider", () => {
    const { monaco, registered } = fakeMonaco();
    registerCompletion(monaco);
    expect(registered).toHaveLength(1);
  });

  it("disposes the previous registration when called again", () => {
    const { monaco, disposed } = fakeMonaco();
    registerCompletion(monaco);
    registerCompletion(monaco);
    registerCompletion(monaco);
    // Each re-registration retires the one before it, so only the newest is live.
    expect(disposed.length).toBe(2);
  });

  it("asks to be triggered on a dot and nothing else", () => {
    const { monaco, registered } = fakeMonaco();
    registerCompletion(monaco);
    expect((registered[0] as { triggerCharacters: string[] }).triggerCharacters).toEqual(["."]);
  });

  it("offers statement openers and snippets for an empty document", async () => {
    const { monaco, registered } = fakeMonaco();
    registerCompletion(monaco);
    const provider = registered[0] as {
      provideCompletionItems: (
        model: unknown,
        position: unknown,
        context: unknown,
        token: unknown,
      ) => Promise<{ suggestions: { label: string }[] }>;
    };

    const model = {
      getValue: () => "",
      getWordUntilPosition: () => ({ startColumn: 1, endColumn: 1 }),
    };
    const result = await provider.provideCompletionItems(
      model,
      { lineNumber: 1, column: 1 },
      {},
      { isCancellationRequested: false },
    );

    const labels = result.suggestions.map((s) => s.label);
    expect(labels).toContain("SELECT");
    expect(labels).toContain("sel");
  });

  it("reports a failure rather than returning an empty list silently", async () => {
    const { monaco, registered } = fakeMonaco();
    registerCompletion(monaco);
    const provider = registered[0] as {
      provideCompletionItems: (
        model: unknown,
        position: unknown,
        context: unknown,
        token: unknown,
      ) => Promise<{ suggestions: { label: string }[] }>;
    };

    const spy = vi.spyOn(console, "error").mockImplementation(() => {});
    const model = {
      getValue: () => {
        throw new Error("boom");
      },
      getWordUntilPosition: () => ({ startColumn: 1, endColumn: 1 }),
    };
    const result = await provider.provideCompletionItems(
      model,
      { lineNumber: 1, column: 1 },
      {},
      { isCancellationRequested: false },
    );

    expect(spy).toHaveBeenCalled();
    expect(result.suggestions.length).toBeGreaterThan(0);
    spy.mockRestore();
  });
});

interface PageLoaderOptions {
  load: (page: number, isCurrent: () => boolean) => Promise<void>;
  hasPage: (page: number) => boolean;
  isCurrent: () => boolean;
  onError: (error: string | null) => void;
  maxConcurrent: number;
  maxQueued: number;
  pageCount: number;
}

export function createPageLoader(options: PageLoaderOptions) {
  const newGeneration = () => ({
    active: true,
    loading: new Set<number>(),
    queue: new Set<number>(),
    failed: new Set<number>(),
    target: 0,
  });
  let generation = newGeneration();
  const current = (state: typeof generation) =>
    state.active && state === generation && options.isCurrent();

  async function fetch(state: typeof generation, page: number) {
    try {
      for (let attempt = 0; attempt < 2 && current(state); attempt++) {
        try {
          await options.load(page, () => current(state));
          return;
        } catch (error) {
          if (attempt === 1 && current(state)) {
            state.failed.add(page);
            if (state.failed.size > options.maxQueued) {
              const oldest = state.failed.values().next().value;
              if (oldest !== undefined) state.failed.delete(oldest);
            }
            options.onError(error instanceof Error ? error.message : String(error));
          }
        }
      }
    } finally {
      state.loading.delete(page);
      if (current(state)) pump(state);
    }
  }

  function pump(state: typeof generation) {
    if (!current(state)) return;
    const pages = [...state.queue].sort(
      (a, b) => Math.abs(a - state.target) - Math.abs(b - state.target),
    );
    for (const page of pages) {
      if (state.loading.size >= options.maxConcurrent) break;
      state.queue.delete(page);
      if (state.loading.has(page) || options.hasPage(page)) continue;
      state.loading.add(page);
      void fetch(state, page);
    }
  }

  return {
    get active() {
      return current(generation);
    },
    get target() {
      return generation.target;
    },
    request(page: number) {
      const state = generation;
      if (!current(state) || !Number.isInteger(page) || page < 0 || page >= options.pageCount)
        return;
      state.target = page;
      if (state.loading.has(page) || state.failed.has(page) || options.hasPage(page)) return;
      state.queue.add(page);
      if (state.queue.size > options.maxQueued) {
        const nearest = [...state.queue].sort((a, b) => Math.abs(a - page) - Math.abs(b - page));
        state.queue = new Set(nearest.slice(0, options.maxQueued));
      }
      pump(state);
    },
    retry() {
      const state = generation;
      if (!current(state)) return;
      options.onError(null);
      for (const page of state.failed) state.queue.add(page);
      state.failed.clear();
      pump(state);
    },
    dispose() {
      generation.active = false;
      generation.queue.clear();
    },
    resume() {
      if (!generation.active) generation = newGeneration();
    },
  };
}

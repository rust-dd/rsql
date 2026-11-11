import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const entries = new Map<string, string>();
const classes = new Set<string>();

beforeEach(() => {
  vi.resetModules();
  entries.clear();
  classes.clear();
  vi.stubGlobal("localStorage", {
    getItem: (key: string) => entries.get(key) ?? null,
    setItem: (key: string, value: string) => entries.set(key, value),
  });
  vi.stubGlobal("document", {
    documentElement: {
      classList: {
        add: (name: string) => classes.add(name),
        remove: (name: string) => classes.delete(name),
        toggle: (name: string, enabled: boolean) => {
          if (enabled) classes.add(name);
          else classes.delete(name);
        },
      },
    },
  });
});

afterEach(() => vi.unstubAllGlobals());

async function restart() {
  vi.resetModules();
  classes.clear();
  return (await import("./ui-store")).useUIStore;
}

describe("theme preference", () => {
  it.each(["light", "dark"] as const)("restores an explicitly selected %s theme", async (theme) => {
    const store = await restart();
    store.getState().setTheme("dark");
    store.getState().setTheme(theme);
    expect(classes.has("dark")).toBe(theme === "dark");

    const restored = await restart();
    expect(restored.getState().theme).toBe(theme);
    expect(classes.has("dark")).toBe(theme === "dark");
  });

  it("persists theme toggles in both directions", async () => {
    const store = await restart();
    store.getState().toggleTheme();
    expect(classes.has("dark")).toBe(true);

    const dark = await restart();
    expect(dark.getState().theme).toBe("dark");
    expect(classes.has("dark")).toBe(true);
    dark.getState().toggleTheme();

    const light = await restart();
    expect(light.getState().theme).toBe("light");
    expect(classes.has("dark")).toBe(false);
  });

  it.each([null, "sepia", "{invalid}"])("defaults to light for saved value %s", async (saved) => {
    if (saved !== null) entries.set("rsql-theme", saved);
    classes.add("dark");
    const store = (await import("./ui-store")).useUIStore;

    expect(store.getState().theme).toBe("light");
    expect(classes.has("dark")).toBe(false);
  });

  it("keeps theme changes usable when storage is unavailable", async () => {
    vi.stubGlobal("localStorage", {
      getItem: () => {
        throw new Error("Storage unavailable");
      },
      setItem: () => {
        throw new Error("Storage unavailable");
      },
    });
    const store = await restart();

    expect(store.getState().theme).toBe("light");
    store.getState().toggleTheme();
    expect(store.getState().theme).toBe("dark");
    expect(classes.has("dark")).toBe(true);
    store.getState().setTheme("light");
    expect(store.getState().theme).toBe("light");
    expect(classes.has("dark")).toBe(false);
  });

  it("does not restore transient UI state with the theme", async () => {
    const store = await restart();
    store.getState().setConnectionModalOpen(true);
    store.getState().pinResult({ columns: ["value"], rows: [["1"]], time: 1 }, "Result");
    store.getState().setTheme("dark");

    const restored = await restart();
    expect(restored.getState().theme).toBe("dark");
    expect(restored.getState().connectionModalOpen).toBe(false);
    expect(restored.getState().pinnedResult).toBeNull();
  });
});

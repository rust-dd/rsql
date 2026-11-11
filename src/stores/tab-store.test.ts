import { describe, expect, it } from "vitest";
import type { Tab } from "@/types";
import { remapTabProjectIds } from "./tab-store";

const tab = (id: string, projectId?: string): Tab => ({
  id,
  type: "query",
  projectId,
  title: id,
  editorValue: "",
  isExecuting: false,
});

describe("remapTabProjectIds", () => {
  it("moves tabs from legacy keys to the new keys and leaves the rest alone", () => {
    const tabs = remapTabProjectIds(
      [tab("t1", "prod"), tab("t2", "uuid-9"), tab("t3")],
      new Map([["prod", "uuid-1"]]),
    );
    expect(tabs.map((t) => t.projectId)).toEqual(["uuid-1", "uuid-9", undefined]);
  });
});

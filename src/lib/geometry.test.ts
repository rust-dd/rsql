import { describe, expect, it } from "vitest";
import { hasGeometryColumn } from "./geometry";

describe("map availability", () => {
  it("does not offer a map for empty or filtered-out rows", () => {
    expect(hasGeometryColumn(["value"], [])).toBe(false);
    expect(hasGeometryColumn(["geometry"], [])).toBe(false);
  });
  it("offers a map only when the sample contains geometry", () => {
    expect(hasGeometryColumn(["value"], [["POINT(19 47)"]])).toBe(true);
    expect(hasGeometryColumn(["geometry"], [[null], ["text"]])).toBe(false);
  });
});

import { describe, expect, it } from "vitest";
import { buildConnection, defaultForm } from "./index";

describe("buildConnection", () => {
  it("keeps the key of an edited connection while taking its new name", () => {
    const connection = buildConnection({ ...defaultForm, name: "  renamed " }, "uuid-1");
    expect(connection).toMatchObject({ id: "uuid-1", name: "renamed" });
  });

  it("gives a new connection a UUID key", () => {
    const { id } = buildConnection({ ...defaultForm, name: "prod" });
    expect(id).toMatch(/^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/);
  });

  it("falls back to the database name when the name is blank", () => {
    const { name } = buildConnection({ ...defaultForm, name: "   ", database: "orders" });
    expect(name).toBe("orders");
  });
});

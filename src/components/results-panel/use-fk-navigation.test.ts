import { describe, expect, it } from "vitest";
import type { ForeignKey } from "@/lib/database-driver";
import { fkTargets } from "./use-fk-navigation";

function fk(
  sourceTable: string,
  sourceColumn: string,
  targetSchema: string,
  targetTable: string,
  targetColumn: string,
): ForeignKey {
  return { sourceTable, sourceColumn, targetSchema, targetTable, targetColumn };
}

describe("fkTargets", () => {
  it("points a column at the schema of the table it references", () => {
    const targets = fkTargets([fk("profiles", "user_id", "auth", "users", "id")], "profiles");
    expect(targets.get("user_id")).toEqual({ schema: "auth", table: "users", column: "id" });
  });

  it("follows the first key listed when a column is in several", () => {
    const targets = fkTargets(
      [
        fk("shipments", "ord", "public", "orders", "id"),
        fk("shipments", "ord", "public", "order_lines", "order_id"),
      ],
      "shipments",
    );
    expect(targets.get("ord")).toEqual({ schema: "public", table: "orders", column: "id" });
  });

  it("ignores keys that belong to other tables", () => {
    const targets = fkTargets(
      [fk("payments", "invoice_id", "public", "invoices", "id")],
      "invoices",
    );
    expect(targets.size).toBe(0);
  });
});

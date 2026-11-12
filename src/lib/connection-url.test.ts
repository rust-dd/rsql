import { describe, expect, it } from "vitest";
import { parseConnectionUrl, SSL_MODES, storedSslMode } from "./connection-url";

const value = (url: string) => {
  const result = parseConnectionUrl(url);
  if ("error" in result) throw new Error(result.error);
  return result.value;
};
const error = (url: string) => {
  const result = parseConnectionUrl(url);
  return "error" in result ? result.error : undefined;
};

describe("parseConnectionUrl", () => {
  it("reads every part of a full URL", () => {
    expect(value("postgresql://alice:secret@db.example.com:6543/orders?sslmode=require")).toEqual({
      host: "db.example.com",
      port: "6543",
      database: "orders",
      username: "alice",
      password: "secret",
      ssl: "require",
    });
  });

  it("accepts the postgres:// scheme and defaults like libpq", () => {
    expect(value("postgres://localhost")).toEqual({
      host: "localhost",
      port: "5432",
      database: "",
      username: "",
      password: "",
      ssl: "prefer",
    });
    expect(value("postgresql:///app").host).toBe("localhost");
  });

  it("decodes credentials and the database name", () => {
    const parsed = value("postgresql://us%40er:p%3Aw%2Fd%25@localhost/sales%20data");
    expect(parsed).toMatchObject({ username: "us@er", password: "p:w/d%", database: "sales data" });
  });

  it("removes IPv6 brackets for the connection API", () => {
    expect(value("postgresql://user:password@[::1]:5433/test")).toMatchObject({
      host: "::1",
      port: "5433",
    });
    expect(value("postgresql://[2001:db8::7]/test").host).toBe("2001:db8::7");
  });

  it.each(SSL_MODES)("keeps sslmode=%s", (mode) => {
    expect(value(`postgresql://localhost/test?sslmode=${mode}`).ssl).toBe(mode);
  });

  it("rejects SSL settings it cannot honor", () => {
    expect(error("postgresql://localhost/test?sslmode=allow")).toMatch(/allow is not supported/);
    expect(error("postgresql://localhost/test?sslmode=strict")).toMatch(/Unknown sslmode "strict"/);
    expect(error("postgresql://localhost/test?ssl=true")).toMatch(/instead of "ssl"/);
  });

  it("rejects options it would otherwise ignore", () => {
    expect(error("postgresql://localhost/test?sslrootcert=/ca.pem")).toMatch(/"sslrootcert"/);
    expect(error("postgresql://localhost/test?sslmode=require&application_name=x")).toMatch(
      /"application_name"/,
    );
  });

  it.each([
    ["postgresql://localhost:0/test", /out of range/],
    ["postgresql://localhost:65536/test", /malformed/],
    ["postgresql://localhost:port/test", /malformed/],
    ["postgresql://host-a,host-b/test", /Multiple hosts/],
    ["postgresql://localhost/%E0%A4%A", /database name contains invalid percent-encoding/],
    ["mysql://localhost/test", /starts with postgresql/],
  ])("rejects %s", (url, message) => {
    expect(error(url)).toMatch(message);
  });
});

describe("storedSslMode", () => {
  it("maps settings saved before SSL modes", () => {
    expect(storedSslMode("true")).toBe("verify-full");
    expect(storedSslMode("false")).toBe("disable");
    expect(storedSslMode(undefined)).toBe("disable");
  });

  it("keeps saved modes and reads unknown values as the strictest mode", () => {
    expect(storedSslMode("verify-ca")).toBe("verify-ca");
    expect(storedSslMode("bogus")).toBe("verify-full");
  });
});

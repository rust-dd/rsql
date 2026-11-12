import { describe, expect, it } from "vitest";
import { sshTunnelParams } from "./ssh-tunnel";

describe("sshTunnelParams", () => {
  it("orders the settings as the connection commands expect", () => {
    const settings = {
      host: "bastion",
      port: "2222",
      user: "ops",
      password: "pw",
      keyPath: "/k",
    };
    expect(sshTunnelParams(settings)).toEqual(["bastion", "2222", "ops", "pw", "/k"]);
  });

  it("defaults an empty port to 22", () => {
    const settings = { host: "bastion", port: "", user: "ops", password: "", keyPath: "" };
    expect(sshTunnelParams(settings)[1]).toBe("22");
  });
});

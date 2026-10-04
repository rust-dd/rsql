import { beforeEach, expect, it, vi } from "vitest";
import { connectWithHostKeyVerification } from "./ssh-trust";

const { ask, invoke } = vi.hoisted(() => ({ ask: vi.fn(), invoke: vi.fn() }));
vi.mock("@tauri-apps/plugin-dialog", () => ({ ask }));
vi.mock("@tauri-apps/api/core", () => ({ invoke }));

const challenge = (previousFingerprint: string | null = null) =>
  `Connection failed: SSH_HOST_KEY:${JSON.stringify({
    id: "approval-id",
    host: "ssh.example",
    port: 2222,
    fingerprint: "SHA256:new-key",
    previousFingerprint,
  })}`;

beforeEach(() => vi.resetAllMocks());

it("persists trust only after approval and retries the connection once", async () => {
  const connect = vi.fn().mockRejectedValueOnce(challenge()).mockResolvedValueOnce("connected");
  ask.mockResolvedValue(true);
  await expect(connectWithHostKeyVerification(connect)).resolves.toBe("connected");
  expect(invoke).toHaveBeenCalledWith("ssh_trust_host_key", { challenge_id: "approval-id" });
  expect(ask.mock.calls[0][0]).toContain("ssh.example:2222");
  expect(ask.mock.calls[0][0]).toContain("SHA256:new-key");
  expect(connect).toHaveBeenCalledTimes(2);
});

it("rejects a cancelled trust decision without persisting or reconnecting", async () => {
  ask.mockResolvedValue(false);
  const connect = vi.fn().mockRejectedValue(challenge());
  await expect(connectWithHostKeyVerification(connect)).rejects.toThrow("not trusted");
  expect(invoke).not.toHaveBeenCalled();
  expect(connect).toHaveBeenCalledTimes(1);
});

it("shows both fingerprints for a changed key and requires explicit replacement", async () => {
  ask.mockResolvedValue(true);
  const connect = vi
    .fn()
    .mockRejectedValueOnce(challenge("SHA256:old-key"))
    .mockResolvedValueOnce("ok");
  await connectWithHostKeyVerification(connect);
  expect(ask.mock.calls[0][0]).toContain("SHA256:old-key");
  expect(ask.mock.calls[0][1].okLabel).toBe("Replace trusted key");
});

it("propagates failed approval and does not reconnect", async () => {
  ask.mockResolvedValue(true);
  invoke.mockRejectedValue(new Error("approval expired"));
  const connect = vi.fn().mockRejectedValue(challenge());
  await expect(connectWithHostKeyVerification(connect)).rejects.toThrow("expired");
  expect(connect).toHaveBeenCalledTimes(1);
});

it.each([
  "ordinary error",
  "SSH_HOST_KEY:{",
  'SSH_HOST_KEY:{"id":"fake"}',
])("fails closed for %s", async (error) => {
  await expect(connectWithHostKeyVerification(vi.fn().mockRejectedValue(error))).rejects.toBe(
    error,
  );
  expect(ask).not.toHaveBeenCalled();
  expect(invoke).not.toHaveBeenCalled();
});

it("does not keep prompting if the key changes again on retry", async () => {
  ask.mockResolvedValue(true);
  const connect = vi.fn().mockRejectedValue(challenge());
  await expect(connectWithHostKeyVerification(connect)).rejects.toBe(challenge());
  expect(connect).toHaveBeenCalledTimes(2);
  expect(ask).toHaveBeenCalledTimes(1);
});

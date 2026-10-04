import { invoke } from "@tauri-apps/api/core";
import { ask } from "@tauri-apps/plugin-dialog";

interface HostKeyChallenge {
  id: string;
  host: string;
  port: number;
  fingerprint: string;
  previousFingerprint: string | null;
}

function parseChallenge(error: unknown): HostKeyChallenge | null {
  const message = String(error);
  const marker = "SSH_HOST_KEY:";
  const start = message.indexOf(marker);
  if (start < 0) return null;
  try {
    const value = JSON.parse(message.slice(start + marker.length));
    if (
      typeof value.id !== "string" ||
      typeof value.host !== "string" ||
      !Number.isInteger(value.port) ||
      value.port < 1 ||
      value.port > 65535 ||
      typeof value.fingerprint !== "string" ||
      !(value.previousFingerprint === null || typeof value.previousFingerprint === "string")
    )
      return null;
    return value;
  } catch {
    return null;
  }
}

export async function connectWithHostKeyVerification<T>(connect: () => Promise<T>): Promise<T> {
  try {
    return await connect();
  } catch (error) {
    const challenge = parseChallenge(error);
    if (!challenge) throw error;
    const changed = challenge.previousFingerprint !== null;
    const message = [
      changed
        ? "The SSH server key has changed. The server may have been replaced or the connection intercepted."
        : "This SSH server is not yet trusted.",
      `Server: ${challenge.host}:${challenge.port}`,
      ...(changed ? [`Previously trusted: ${challenge.previousFingerprint}`] : []),
      `New fingerprint: ${challenge.fingerprint}`,
      "Verify this fingerprint with your server administrator before trusting it.",
    ].join("\n\n");
    const approved = await ask(message, {
      title: changed ? "SSH server key changed" : "Verify SSH server",
      kind: "warning",
      okLabel: changed ? "Replace trusted key" : "Trust this key",
      cancelLabel: "Cancel connection",
    });
    if (!approved) throw new Error("SSH connection cancelled: server key was not trusted.");
    await invoke("ssh_trust_host_key", { challenge_id: challenge.id });
    return connect();
  }
}

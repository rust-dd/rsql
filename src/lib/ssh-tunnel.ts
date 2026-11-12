export interface SshTunnelSettings {
  host: string;
  port: string;
  user: string;
  password: string;
  keyPath: string;
}

/** The `[host, port, user, password, key_path]` list the Rust connection commands take. */
export function sshTunnelParams(ssh: SshTunnelSettings): string[] {
  return [ssh.host, ssh.port || "22", ssh.user, ssh.password, ssh.keyPath];
}

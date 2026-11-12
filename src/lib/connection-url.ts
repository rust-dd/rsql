export const SSL_MODES = ["disable", "prefer", "require", "verify-ca", "verify-full"] as const;
export type SslMode = (typeof SSL_MODES)[number];

export const SSL_MODE_LABELS: Record<SslMode, string> = {
  disable: "Disable — no TLS",
  prefer: "Prefer — TLS if available, unverified",
  require: "Require — TLS, unverified",
  "verify-ca": "Verify CA — trusted certificate",
  "verify-full": "Verify full — trusted certificate and host name",
};

const isSslMode = (value: string): value is SslMode =>
  (SSL_MODES as readonly string[]).includes(value);

/** Connections saved before SSL modes stored "true" (a verified certificate) or "false". */
export function storedSslMode(value: string | undefined): SslMode {
  if (!value || value === "false") return "disable";
  return isSslMode(value) ? value : "verify-full";
}

export interface ParsedConnectionUrl {
  host: string;
  port: string;
  database: string;
  username: string;
  password: string;
  ssl: SslMode;
}

export type ConnectionUrlResult = { value: ParsedConnectionUrl } | { error: string };

function decode(component: string, what: string): string {
  try {
    return decodeURIComponent(component);
  } catch {
    throw new Error(`The ${what} contains invalid percent-encoding.`);
  }
}

function sslModeOf(params: URLSearchParams): SslMode {
  const unsupported = [...params.keys()].filter((key) => key !== "sslmode");
  if (unsupported.includes("ssl")) {
    throw new Error('Use sslmode=require or sslmode=verify-full instead of "ssl".');
  }
  if (unsupported.length > 0) {
    throw new Error(
      `Unsupported connection option "${unsupported[0]}". Remove it and set the connection up in the form.`,
    );
  }
  const mode = params.get("sslmode") ?? "prefer";
  if (mode === "allow")
    throw new Error("sslmode=allow is not supported. Use disable, prefer or require.");
  if (!isSslMode(mode))
    throw new Error(`Unknown sslmode "${mode}". Use one of: ${SSL_MODES.join(", ")}.`);
  return mode;
}

/** Parses a libpq connection URL; anything the app would silently change is an error instead. */
export function parseConnectionUrl(input: string): ConnectionUrlResult {
  const url = input.trim();
  if (!/^postgres(?:ql)?:\/\//i.test(url)) {
    return { error: "A connection URL starts with postgresql:// or postgres://." };
  }
  let parsed: URL;
  try {
    parsed = new URL(url);
  } catch {
    return { error: "The connection URL is malformed. Check the host and port." };
  }
  try {
    const host = parsed.hostname.replace(/^\[(.*)\]$/, "$1");
    if (host.includes(",")) throw new Error("Multiple hosts are not supported.");
    const port = parsed.port || "5432";
    if (!/^\d+$/.test(port) || Number(port) < 1 || Number(port) > 65535) {
      throw new Error(`Port ${port} is out of range.`);
    }
    return {
      value: {
        host: host ? decode(host, "host") : "localhost",
        port,
        database: decode(parsed.pathname.replace(/^\//, ""), "database name"),
        username: decode(parsed.username, "user name"),
        password: decode(parsed.password, "password"),
        ssl: sslModeOf(parsed.searchParams),
      },
    };
  } catch (error) {
    return { error: error instanceof Error ? error.message : String(error) };
  }
}

import { CheckCircle2, Loader2, XCircle } from "lucide-react";
import type React from "react";
import { useEffect, useState } from "react";
import { pgsqlTestConnection } from "@/tauri";
import type { DriverType, ProjectDetails } from "@/types";
import { Button } from "../ui/button";
import { Dialog, DialogContent, DialogDescription, DialogHeader, DialogTitle } from "../ui/dialog";
import {
  ConnStringField,
  DatabaseField,
  DriverDisplay,
  HostPortFields,
  NameField,
  PasswordField,
  SslCheckbox,
  UsernameField,
} from "./form-fields";
import { SshConfig } from "./ssh-config";

interface ConnectionModalProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onSave: (connection: ConnectionConfig) => void;
  editData?: { id: string; details: ProjectDetails } | null;
}

export interface ConnectionConfig {
  id: string;
  name: string;
  driver: DriverType;
  host: string;
  port: string;
  database: string;
  username: string;
  password: string;
  ssl: boolean;
  sshEnabled: boolean;
  sshHost: string;
  sshPort: string;
  sshUser: string;
  sshPassword: string;
  sshKeyPath: string;
}

export const defaultForm: Omit<ConnectionConfig, "id"> = {
  name: "",
  driver: "PGSQL",
  host: "localhost",
  port: "5432",
  database: "",
  username: "",
  password: "",
  ssl: false,
  sshEnabled: false,
  sshHost: "",
  sshPort: "22",
  sshUser: "",
  sshPassword: "",
  sshKeyPath: "",
};

function parseConnectionString(url: string): Partial<Omit<ConnectionConfig, "id">> | null {
  try {
    // Handle postgresql:// and postgres:// schemes
    const normalized = url.trim().replace(/^postgres:\/\//, "postgresql://");
    if (!normalized.startsWith("postgresql://")) return null;
    const parsed = new URL(normalized);
    const params = parsed.searchParams;
    const ssl =
      params.get("sslmode") === "require" ||
      params.get("sslmode") === "verify-full" ||
      params.get("ssl") === "true";
    return {
      driver: "PGSQL",
      host: parsed.hostname || "localhost",
      port: parsed.port || "5432",
      database: parsed.pathname.replace(/^\//, "") || "",
      username: decodeURIComponent(parsed.username || ""),
      password: decodeURIComponent(parsed.password || ""),
      ssl,
    };
  } catch {
    return null;
  }
}

/** An edited connection keeps its key, so renaming never orphans its tabs or saved queries. */
export function buildConnection(
  form: Omit<ConnectionConfig, "id">,
  editId?: string,
): ConnectionConfig {
  const name = form.name.trim() || form.database || form.host;
  return { ...form, id: editId ?? crypto.randomUUID(), name };
}

export function ConnectionModal({ open, onOpenChange, onSave, editData }: ConnectionModalProps) {
  const [formData, setFormData] = useState<Omit<ConnectionConfig, "id">>(defaultForm);
  const [connString, setConnString] = useState("");
  const [connStringError, setConnStringError] = useState(false);
  const [testing, setTesting] = useState(false);
  const [testResult, setTestResult] = useState<{ ok: boolean; message: string } | null>(null);

  useEffect(() => {
    if (open && editData) {
      setFormData({
        name: editData.details.name || editData.id,
        driver: editData.details.driver,
        host: editData.details.host,
        port: editData.details.port,
        database: editData.details.database,
        username: editData.details.username,
        password: editData.details.password,
        ssl: editData.details.ssl === "true",
        sshEnabled: editData.details.sshEnabled === "true",
        sshHost: editData.details.sshHost || "",
        sshPort: editData.details.sshPort || "22",
        sshUser: editData.details.sshUser || "",
        sshPassword: editData.details.sshPassword || "",
        sshKeyPath: editData.details.sshKeyPath || "",
      });
      setConnString("");
      setConnStringError(false);
      setTestResult(null);
    } else if (open && !editData) {
      setFormData(defaultForm);
      setConnString("");
      setConnStringError(false);
      setTestResult(null);
    }
  }, [open, editData]);

  const handleConnStringPaste = (value: string) => {
    setConnString(value);
    setConnStringError(false);
    if (!value.trim()) return;
    const parsed = parseConnectionString(value);
    if (parsed) {
      setFormData((prev) => ({ ...prev, ...parsed, name: prev.name || parsed.database || "" }));
    } else {
      setConnStringError(true);
    }
  };

  const isEditing = !!editData;

  const handleTestConnection = async () => {
    setTesting(true);
    setTestResult(null);
    try {
      const key: [string, string, string, string, string, string] = [
        formData.username,
        formData.password,
        formData.database,
        formData.host,
        formData.port,
        formData.ssl ? "true" : "false",
      ];
      const version = await pgsqlTestConnection(key);
      setTestResult({ ok: true, message: version });
    } catch (err: unknown) {
      const msg = err instanceof Error ? err.message : String(err);
      setTestResult({ ok: false, message: msg });
    } finally {
      setTesting(false);
    }
  };

  const handleSubmit = (e: React.FormEvent) => {
    e.preventDefault();
    onSave(buildConnection(formData, editData?.id));
    onOpenChange(false);
  };

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="bg-card border-border/50 rounded-xl sm:max-w-[500px]">
        <DialogHeader>
          <DialogTitle className="font-mono text-foreground">
            {isEditing ? "Edit Connection" : "New Connection"}
          </DialogTitle>
          <DialogDescription className="text-muted-foreground text-xs">
            {isEditing ? "Update connection details" : "Add a new database connection"}
          </DialogDescription>
        </DialogHeader>
        <form onSubmit={handleSubmit} className="space-y-4 mt-4">
          {!isEditing && (
            <ConnStringField
              value={connString}
              onChange={handleConnStringPaste}
              error={connStringError}
            />
          )}

          <DriverDisplay driver={formData.driver} />

          <NameField
            value={formData.name}
            onChange={(value) => setFormData({ ...formData, name: value })}
          />

          <HostPortFields
            host={formData.host}
            port={formData.port}
            onHostChange={(value) => setFormData({ ...formData, host: value })}
            onPortChange={(value) => setFormData({ ...formData, port: value })}
          />

          <DatabaseField
            value={formData.database}
            onChange={(value) => setFormData({ ...formData, database: value })}
          />

          <UsernameField
            value={formData.username}
            onChange={(value) => setFormData({ ...formData, username: value })}
          />

          <PasswordField
            value={formData.password}
            onChange={(value) => setFormData({ ...formData, password: value })}
          />

          <SslCheckbox
            checked={formData.ssl}
            onChange={(checked) => setFormData({ ...formData, ssl: checked })}
          />

          <SshConfig
            enabled={formData.sshEnabled}
            sshHost={formData.sshHost}
            sshPort={formData.sshPort}
            sshUser={formData.sshUser}
            sshPassword={formData.sshPassword}
            sshKeyPath={formData.sshKeyPath}
            onEnabledChange={(checked) => setFormData({ ...formData, sshEnabled: checked })}
            onSshHostChange={(value) => setFormData({ ...formData, sshHost: value })}
            onSshPortChange={(value) => setFormData({ ...formData, sshPort: value })}
            onSshUserChange={(value) => setFormData({ ...formData, sshUser: value })}
            onSshPasswordChange={(value) => setFormData({ ...formData, sshPassword: value })}
            onSshKeyPathChange={(value) => setFormData({ ...formData, sshKeyPath: value })}
          />

          {testResult && (
            <div
              className={`flex items-start gap-2 rounded-lg border px-3 py-2 text-xs font-mono ${
                testResult.ok
                  ? "border-emerald-500/30 bg-emerald-500/5 text-emerald-500"
                  : "border-destructive/30 bg-destructive/5 text-destructive"
              }`}
            >
              {testResult.ok ? (
                <CheckCircle2 className="h-3.5 w-3.5 shrink-0 mt-0.5" />
              ) : (
                <XCircle className="h-3.5 w-3.5 shrink-0 mt-0.5" />
              )}
              <span className="break-all">{testResult.message}</span>
            </div>
          )}

          <div className="flex justify-between pt-4">
            <Button
              type="button"
              variant="outline"
              onClick={() => void handleTestConnection()}
              disabled={testing || !formData.host || !formData.database}
              className="font-mono text-xs"
            >
              {testing && <Loader2 className="h-3 w-3 animate-spin mr-1.5" />}
              Test Connection
            </Button>
            <div className="flex gap-2">
              <Button
                type="button"
                variant="ghost"
                onClick={() => onOpenChange(false)}
                className="font-mono text-xs"
              >
                Cancel
              </Button>
              <Button type="submit" variant="gradient" className="font-mono text-xs">
                {isEditing ? "Save Changes" : "Connect"}
              </Button>
            </div>
          </div>
        </form>
      </DialogContent>
    </Dialog>
  );
}

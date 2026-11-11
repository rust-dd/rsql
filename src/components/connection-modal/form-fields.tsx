import { DRIVER_CONFIGS } from "@/lib/database-driver";
import type { DriverType } from "@/types";
import { Input } from "../ui/input";
import { Label } from "../ui/label";

interface ConnStringFieldProps {
  value: string;
  onChange: (value: string) => void;
  error: boolean;
}

export function ConnStringField({ value, onChange, error }: ConnStringFieldProps) {
  return (
    <div className="space-y-2">
      <Label htmlFor="connString" className="font-mono text-xs text-foreground">
        Connection URL
      </Label>
      <Input
        id="connString"
        value={value}
        onChange={(e) => onChange(e.target.value)}
        placeholder="postgresql://user:password@host:5432/database"
        className={`bg-input/80 border-border/50 text-foreground font-mono text-sm rounded-lg ${error ? "border-destructive" : ""}`}
      />
      {error && (
        <p className="text-destructive text-[11px] font-mono">Invalid connection URL format</p>
      )}
      <div className="relative">
        <div className="absolute inset-0 flex items-center">
          <span className="w-full border-t border-border/30" />
        </div>
        <div className="relative flex justify-center text-[10px]">
          <span className="bg-card px-2 text-muted-foreground">or fill in manually</span>
        </div>
      </div>
    </div>
  );
}

interface DriverDisplayProps {
  driver: DriverType;
}

export function DriverDisplay({ driver }: DriverDisplayProps) {
  return (
    <div className="space-y-2">
      <Label htmlFor="driver" className="font-mono text-xs text-foreground">
        Database Type
      </Label>
      <div className="w-full bg-input/80 border border-border/50 text-foreground font-mono text-sm rounded-lg px-3 py-2">
        {DRIVER_CONFIGS[driver].name}
      </div>
    </div>
  );
}

interface NameFieldProps {
  value: string;
  onChange: (value: string) => void;
}

export function NameField({ value, onChange }: NameFieldProps) {
  return (
    <div className="space-y-2">
      <Label htmlFor="name" className="font-mono text-xs text-foreground">
        Connection Name
      </Label>
      <Input
        id="name"
        value={value}
        onChange={(e) => onChange(e.target.value)}
        placeholder="production-db"
        required
        className="bg-input/80 border-border/50 text-foreground font-mono text-sm rounded-lg"
      />
    </div>
  );
}

interface HostPortFieldsProps {
  host: string;
  port: string;
  onHostChange: (value: string) => void;
  onPortChange: (value: string) => void;
}

export function HostPortFields({ host, port, onHostChange, onPortChange }: HostPortFieldsProps) {
  return (
    <div className="grid grid-cols-2 gap-4">
      <div className="space-y-2">
        <Label htmlFor="host" className="font-mono text-xs text-foreground">
          Host
        </Label>
        <Input
          id="host"
          value={host}
          onChange={(e) => onHostChange(e.target.value)}
          placeholder="localhost"
          required
          className="bg-input/80 border-border/50 text-foreground font-mono text-sm rounded-lg"
        />
      </div>
      <div className="space-y-2">
        <Label htmlFor="port" className="font-mono text-xs text-foreground">
          Port
        </Label>
        <Input
          id="port"
          value={port}
          onChange={(e) => onPortChange(e.target.value)}
          placeholder="5432"
          required
          className="bg-input/80 border-border/50 text-foreground font-mono text-sm rounded-lg"
        />
      </div>
    </div>
  );
}

interface DatabaseFieldProps {
  value: string;
  onChange: (value: string) => void;
}

export function DatabaseField({ value, onChange }: DatabaseFieldProps) {
  return (
    <div className="space-y-2">
      <Label htmlFor="database" className="font-mono text-xs text-foreground">
        Database
      </Label>
      <Input
        id="database"
        value={value}
        onChange={(e) => onChange(e.target.value)}
        placeholder="mydb"
        required
        className="bg-input/80 border-border/50 text-foreground font-mono text-sm rounded-lg"
      />
    </div>
  );
}

interface UsernameFieldProps {
  value: string;
  onChange: (value: string) => void;
}

export function UsernameField({ value, onChange }: UsernameFieldProps) {
  return (
    <div className="space-y-2">
      <Label htmlFor="username" className="font-mono text-xs text-foreground">
        Username
      </Label>
      <Input
        id="username"
        value={value}
        onChange={(e) => onChange(e.target.value)}
        placeholder="postgres"
        required
        className="bg-input/80 border-border/50 text-foreground font-mono text-sm rounded-lg"
      />
    </div>
  );
}

interface PasswordFieldProps {
  value: string;
  onChange: (value: string) => void;
}

export function PasswordField({ value, onChange }: PasswordFieldProps) {
  return (
    <div className="space-y-2">
      <Label htmlFor="password" className="font-mono text-xs text-foreground">
        Password
      </Label>
      <Input
        id="password"
        type="password"
        value={value}
        onChange={(e) => onChange(e.target.value)}
        placeholder="••••••••"
        className="bg-input/80 border-border/50 text-foreground font-mono text-sm rounded-lg"
      />
    </div>
  );
}

interface SslCheckboxProps {
  checked: boolean;
  onChange: (checked: boolean) => void;
}

export function SslCheckbox({ checked, onChange }: SslCheckboxProps) {
  return (
    <div className="flex items-center gap-2">
      <input
        type="checkbox"
        id="ssl"
        checked={checked}
        onChange={(e) => onChange(e.target.checked)}
        className="h-4 w-4 rounded border-border bg-input"
      />
      <Label htmlFor="ssl" className="font-mono text-xs text-foreground cursor-pointer">
        Use SSL
      </Label>
    </div>
  );
}

import { Database } from "lucide-react";

export function Logo() {
  return (
    <span className="brand">
      <span className="brand-mark">
        <Database size={19} strokeWidth={1.8} aria-hidden="true" />
      </span>
      <span>
        RSQL<span className="brand-period">.</span>
      </span>
    </span>
  );
}

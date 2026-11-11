import { ChevronDown, ChevronRight, Loader2 } from "lucide-react";
import type React from "react";
import { cn } from "@/lib/utils";
import { IndentGuides } from "./indent-guides";

export function TreeRow({
  indent,
  icon,
  label,
  bold,
  expanded,
  loading: isLoading,
  trailing,
  selected,
  onClick,
  onDoubleClick,
  onContextMenu,
}: {
  indent: number;
  icon: React.ReactNode;
  label: string;
  bold?: boolean;
  expanded?: boolean;
  loading?: boolean;
  trailing?: React.ReactNode;
  selected?: boolean;
  onClick?: () => void;
  onDoubleClick?: () => void;
  onContextMenu?: (e: React.MouseEvent) => void;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      onDoubleClick={onDoubleClick}
      onContextMenu={onContextMenu}
      className={cn(
        "relative flex w-full min-w-0 items-center gap-1 overflow-hidden py-1 text-left text-sm transition-colors rounded-sm whitespace-nowrap",
        selected
          ? "bg-primary/10 text-foreground"
          : "hover:bg-white/[0.06] dark:hover:bg-white/[0.06] hover:bg-black/[0.04]",
      )}
      style={{ paddingLeft: `${indent}px` }}
    >
      <IndentGuides indent={indent} />
      {expanded !== undefined ? (
        isLoading ? (
          <Loader2 className="h-3 w-3 shrink-0 animate-spin text-muted-foreground" />
        ) : expanded ? (
          <ChevronDown className="h-3 w-3 shrink-0 text-muted-foreground" />
        ) : (
          <ChevronRight className="h-3 w-3 shrink-0 text-muted-foreground" />
        )
      ) : null}
      <span className="shrink-0">{icon}</span>
      <span
        className={cn("min-w-0 flex-1 truncate font-mono text-xs", bold && "font-semibold")}
        title={label}
      >
        {label}
      </span>
      {trailing && (
        <span className="ml-auto mr-1 max-w-[max(4rem,40%)] shrink-0 truncate">{trailing}</span>
      )}
    </button>
  );
}

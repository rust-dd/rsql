import { ChevronDown, ChevronRight, Loader2 } from "lucide-react";
import type React from "react";
import { cn } from "@/lib/utils";
import { CHEVRON_WIDTH, indentPx } from "./constants";
import { IndentGuides } from "./indent-guides";

export function TreeRow({
  depth,
  icon,
  label,
  detail,
  bold,
  muted,
  expanded,
  loading: isLoading,
  trailing,
  selected,
  onClick,
  onDoubleClick,
  onContextMenu,
}: {
  depth: number;
  icon: React.ReactNode;
  label: string;
  detail?: React.ReactNode;
  bold?: boolean;
  muted?: boolean;
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
        "relative flex h-[22px] w-full min-w-0 items-center gap-1 overflow-hidden rounded-sm pr-1.5 text-left whitespace-nowrap transition-colors",
        selected ? "bg-primary/10 text-foreground" : "hover:bg-sidebar-accent",
      )}
      style={{ paddingLeft: `${indentPx(depth)}px` }}
    >
      <IndentGuides depth={depth} />
      {/* Reserved even on leaves so sibling icons and labels stay in one column. */}
      <span
        className="flex shrink-0 items-center justify-center text-muted-foreground"
        style={{ width: `${CHEVRON_WIDTH}px` }}
      >
        {expanded === undefined ? null : isLoading ? (
          <Loader2 className="h-3 w-3 animate-spin" />
        ) : expanded ? (
          <ChevronDown className="h-3 w-3" />
        ) : (
          <ChevronRight className="h-3 w-3" />
        )}
      </span>
      <span className="flex h-3.5 w-3.5 shrink-0 items-center justify-center">{icon}</span>
      <span className="flex min-w-0 flex-1 items-baseline gap-1.5 overflow-hidden pl-0.5">
        {/* Never shrinks: any sub-pixel squeeze would ellipsize the name before the detail. */}
        <span
          className={cn(
            "max-w-full shrink-0 truncate font-mono text-xs",
            bold && "font-semibold",
            muted ? "text-muted-foreground" : "text-sidebar-foreground",
          )}
          title={label}
        >
          {label}
        </span>
        {detail !== undefined && (
          <span
            className="min-w-0 shrink-[999] truncate font-mono text-[10px] text-muted-foreground/80"
            title={typeof detail === "string" ? detail : undefined}
          >
            {detail}
          </span>
        )}
      </span>
      {trailing && (
        <span className="ml-1 flex max-w-[45%] shrink-0 items-center gap-1.5">{trailing}</span>
      )}
    </button>
  );
}

export function StatusDot({ className }: { className?: string }) {
  return <span className={cn("h-1.5 w-1.5 shrink-0 rounded-full", className)} />;
}

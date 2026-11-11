import type React from "react";
import { TreeRow } from "./tree-row";

export function SectionHeader({
  depth,
  label,
  count,
  icon,
  expanded,
  loading,
  onClick,
}: {
  depth: number;
  label: string;
  count?: number;
  icon: React.ReactNode;
  expanded: boolean;
  loading?: boolean;
  onClick: () => void;
}) {
  return (
    <TreeRow
      depth={depth}
      icon={icon}
      label={label}
      muted
      detail={count === undefined ? undefined : String(count)}
      expanded={expanded}
      loading={loading}
      onClick={onClick}
    />
  );
}

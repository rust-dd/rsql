export const DEPTH = {
  server: 0,
  cat: 1,
  db: 2,
  schema: 3,
  schemaObj: 4,
  table: 5,
  section: 6,
  item: 7,
};

const INDENT_BASE = 4;
const INDENT_STEP = 8;
export const CHEVRON_WIDTH = 12;

export const indentPx = (depth: number) => INDENT_BASE + depth * INDENT_STEP;

export const SECTION_ICON = "h-3.5 w-3.5 text-muted-foreground";
export const LEAF_ICON = "h-3 w-3 text-muted-foreground/60";

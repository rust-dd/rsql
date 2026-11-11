import { CHEVRON_WIDTH, indentPx } from "./constants";

/** One line per ancestor, centred under that ancestor's chevron. */
export function IndentGuides({ depth }: { depth: number }) {
  const lefts = Array.from({ length: depth }, (_, level) => indentPx(level) + CHEVRON_WIDTH / 2);
  return (
    <>
      {lefts.map((left) => (
        <span key={left} className="sidebar-indent-guide" style={{ left: `${left}px` }} />
      ))}
    </>
  );
}

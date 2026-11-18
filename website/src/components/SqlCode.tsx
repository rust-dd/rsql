const keywords =
  /^(SELECT|FROM|WHERE|GROUP|BY|ORDER|DESC|ASC|AS|JOIN|ON|LEFT|LIMIT|COUNT|AVG|ROUND|SUM|TRUE|FALSE|EXPLAIN|ANALYZE)$/i;

export function SqlCode({ sql }: { sql: string }) {
  const tokens = Array.from(
    sql.matchAll(/--[^\n]*|'(?:[^']|'')*'|\b\d+(?:\.\d+)?\b|\b\w+\b|[^\w]/g),
  );
  return (
    <code>
      {tokens.map((token) => {
        const value = token[0];
        const kind = value.startsWith("--")
          ? "comment"
          : value.startsWith("'")
            ? "string"
            : keywords.test(value)
              ? "keyword"
              : /^\d/.test(value)
                ? "number"
                : "plain";
        return (
          <span key={token.index} className={`sql-${kind}`}>
            {value}
          </span>
        );
      })}
    </code>
  );
}

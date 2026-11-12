export interface SqlStatement {
  text: string;
  start: number;
  /** Offset after the last token; the terminating semicolon is excluded. */
  end: number;
  /** Offset of the terminating semicolon, if the statement has one. */
  terminator?: number;
}

export interface TextSelection {
  start: number;
  end: number;
}

const IDENT_START = /[A-Za-z_]|\P{ASCII}/u;
const IDENT_PART = /[A-Za-z_0-9$]|\P{ASCII}/u;
const DOLLAR_TAG = /\$(?:(?:[A-Za-z_]|\P{ASCII})(?:[A-Za-z_0-9]|\P{ASCII})*)?\$/uy;

function dollarTagAt(sql: string, at: number): string | undefined {
  DOLLAR_TAG.lastIndex = at;
  return DOLLAR_TAG.exec(sql)?.[0];
}

function skipBlockComment(sql: string, from: number): number {
  let depth = 0;
  for (let i = from; i < sql.length; i++) {
    if (sql.startsWith("/*", i)) {
      depth++;
      i++;
    } else if (sql.startsWith("*/", i)) {
      depth--;
      i++;
      if (depth === 0) return i + 1;
    }
  }
  return sql.length;
}

function skipQuoted(sql: string, from: number, quote: string, backslashEscapes: boolean): number {
  for (let i = from + 1; i < sql.length; i++) {
    if (backslashEscapes && sql[i] === "\\") i++;
    else if (sql[i] === quote) {
      if (sql[i + 1] !== quote) return i + 1;
      i++;
    }
  }
  return sql.length;
}

function isRoutineDefinition(words: string[]): boolean {
  const [first, second, third, fourth] = words;
  const routine = (word?: string) => word === "function" || word === "procedure";
  return (
    first === "create" &&
    (routine(second) || (second === "or" && third === "replace" && routine(fourth)))
  );
}

/**
 * Splits a script the way psql does: semicolons inside literals, comments, parentheses
 * and the BEGIN ... END body of a CREATE FUNCTION or PROCEDURE do not end a statement.
 */
export function splitStatements(sql: string): SqlStatement[] {
  const statements: SqlStatement[] = [];
  let start = -1;
  let end = -1;
  let parens = 0;
  let blockDepth = 0;
  let words: string[] = [];

  const token = (from: number, to: number) => {
    if (start < 0) start = from;
    end = to;
  };
  const finish = (terminator?: number) => {
    if (start >= 0) statements.push({ text: sql.slice(start, end), start, end, terminator });
    start = end = -1;
    parens = blockDepth = 0;
    words = [];
  };

  let i = 0;
  while (i < sql.length) {
    const char = sql[i];
    if (/\s/.test(char)) {
      i++;
    } else if (sql.startsWith("--", i)) {
      const newline = sql.indexOf("\n", i);
      i = newline < 0 ? sql.length : newline + 1;
    } else if (sql.startsWith("/*", i)) {
      i = skipBlockComment(sql, i);
    } else if (char === ";" && parens === 0 && blockDepth === 0) {
      finish(i);
      i++;
    } else if (char === "'" || char === '"') {
      const to = skipQuoted(sql, i, char, false);
      token(i, to);
      i = to;
    } else if (char === "$" && dollarTagAt(sql, i)) {
      const tag = dollarTagAt(sql, i) as string;
      const close = sql.indexOf(tag, i + tag.length);
      const to = close < 0 ? sql.length : close + tag.length;
      token(i, to);
      i = to;
    } else if (IDENT_START.test(char)) {
      let to = i + 1;
      while (to < sql.length && IDENT_PART.test(sql[to])) to++;
      const word = sql.slice(i, to).toLowerCase();
      if (word === "e" && sql[to] === "'") {
        to = skipQuoted(sql, to, "'", true);
      } else {
        words.push(word);
        if (parens === 0 && isRoutineDefinition(words.slice(0, 4))) {
          if (word === "begin") blockDepth++;
          else if (word === "case" && blockDepth > 0) blockDepth++;
          else if (word === "end" && blockDepth > 0) blockDepth--;
        }
      }
      token(i, to);
      i = to;
    } else {
      if (char === "(") parens++;
      else if (char === ")") parens = Math.max(0, parens - 1);
      token(i, i + 1);
      i++;
    }
  }
  finish();
  return statements;
}

export type StatementTarget = { statement: SqlStatement } | { error: string };

/** Picks the selected statement, or the one at the caret; anything ambiguous is an error. */
export function targetStatement(sql: string, selection?: TextSelection): StatementTarget {
  if (selection && selection.start !== selection.end) {
    const from = Math.min(selection.start, selection.end);
    const selected = splitStatements(sql.slice(from, Math.max(selection.start, selection.end)));
    if (selected.length === 0) return { error: "The selection contains no SQL statement." };
    if (selected.length > 1) {
      return {
        error: `The selection contains ${selected.length} statements. Select one statement.`,
      };
    }
    const [only] = selected;
    const shift = (offset?: number) => (offset === undefined ? undefined : offset + from);
    return {
      statement: {
        ...only,
        start: from + only.start,
        end: from + only.end,
        terminator: shift(only.terminator),
      },
    };
  }
  const statements = splitStatements(sql);
  if (statements.length === 0) return { error: "There is no SQL statement to run." };
  if (statements.length === 1) return { statement: statements[0] };
  const caret = selection?.start ?? -1;
  const atCaret = statements.find(
    (s) => s.start <= caret && caret <= (s.terminator === undefined ? s.end : s.terminator + 1),
  );
  return atCaret
    ? { statement: atCaret }
    : { error: "The editor contains several statements. Place the cursor in one or select it." };
}

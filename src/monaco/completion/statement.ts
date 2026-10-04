import type { CompletionRange } from "./types";

/** Limit completion work to the caret's statement without splitting quoted SQL. */
export function statementAt(sql: string, caret: { lineNumber: number; column: number }) {
  let caretOffset = 0;
  for (let line = 1; line < caret.lineNumber; line++) {
    const end = sql.indexOf("\n", caretOffset);
    if (end < 0) break;
    caretOffset = end + 1;
  }
  caretOffset += caret.column - 1;
  let start = 0;
  let end = sql.length;
  let quote = "";
  let escaped = false;
  let dollar = "";
  let commentDepth = 0;
  let lineComment = false;
  let parentheses = 0;
  for (let i = 0; i < sql.length; i++) {
    const char = sql[i];
    const pair = sql.slice(i, i + 2);
    if (lineComment) {
      if (char === "\n") lineComment = false;
      continue;
    }
    if (commentDepth) {
      if (pair === "/*") {
        commentDepth++;
        i++;
      } else if (pair === "*/") {
        commentDepth--;
        i++;
      }
      continue;
    }
    if (dollar) {
      if (sql.startsWith(dollar, i)) {
        i += dollar.length - 1;
        dollar = "";
      }
      continue;
    }
    if (quote) {
      if (char === "\\" && escaped) i++;
      else if (char === quote) {
        if (sql[i + 1] === quote) i++;
        else quote = "";
      }
      continue;
    }
    if (pair === "--") {
      lineComment = true;
      i++;
    } else if (pair === "/*") {
      commentDepth = 1;
      i++;
    } else if (char === "'" || char === '"') {
      quote = char;
      escaped = char === "'" && /[eE]/.test(sql[i - 1] ?? "") && !/[\w$]/.test(sql[i - 2] ?? "");
    } else if (char === "$" && !/[\w$]/.test(sql[i - 1] ?? "")) {
      const tag = /^\$(?:[A-Za-z_][A-Za-z_0-9]*)?\$/.exec(sql.slice(i));
      if (tag) {
        dollar = tag[0];
        i += dollar.length - 1;
      }
    } else if (char === "(") parentheses++;
    else if (char === ")") parentheses = Math.max(0, parentheses - 1);
    else if (char === ";" && parentheses === 0) {
      if (i >= caretOffset) {
        end = i;
        break;
      }
      start = i + 1;
    }
  }
  // Unquoted SQL function bodies can contain statement terminators of their own.
  if (/\bBEGIN\s+ATOMIC\b/i.test(sql)) {
    start = 0;
    end = sql.length;
  }
  const before = sql.slice(0, start);
  const lineOffset = before.split("\n").length - 1;
  const columnOffset = start - (before.lastIndexOf("\n") + 1);
  const position = (lineNumber: number, column: number) => ({
    lineNumber: lineNumber - lineOffset,
    column: lineNumber === lineOffset + 1 ? column - columnOffset : column,
  });
  return {
    sql: sql.slice(start, end),
    caret: position(caret.lineNumber, caret.column),
    localRange: (range: CompletionRange): CompletionRange => {
      const a = position(range.startLineNumber, range.startColumn);
      const b = position(range.endLineNumber, range.endColumn);
      return {
        startLineNumber: a.lineNumber,
        startColumn: a.column,
        endLineNumber: b.lineNumber,
        endColumn: b.column,
      };
    },
    globalRange: (range: CompletionRange): CompletionRange => ({
      startLineNumber: range.startLineNumber + lineOffset,
      endLineNumber: range.endLineNumber + lineOffset,
      startColumn: range.startColumn + (range.startLineNumber === 1 ? columnOffset : 0),
      endColumn: range.endColumn + (range.endLineNumber === 1 ? columnOffset : 0),
    }),
  };
}

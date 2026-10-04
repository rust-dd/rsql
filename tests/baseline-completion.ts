import { PostgreSQL } from "dt-sql-parser";
import type { ParseRequest } from "../src/monaco/completion/parse-types";
import { readExpectation, readScope } from "../src/monaco/completion/service";

/** Reproduces the pre-worker completion path for the browser benchmark. */
export function createBaselineParser() {
  const parser = new PostgreSQL();
  return ({ sql, caret, wordRange }: ParseRequest) => {
    const suggestion = parser.getSuggestionAtCaretPosition(sql, caret);
    return suggestion
      ? {
          expectation: readExpectation(suggestion.syntax as never, caret, wordRange),
          keywords: suggestion.keywords,
          scope: readScope(parser.getAllEntities(sql, caret) as never),
        }
      : null;
  };
}

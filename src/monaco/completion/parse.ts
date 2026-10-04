import { PostgreSQL } from "dt-sql-parser";
import type { ParsedCompletion, ParseRequest } from "./parse-types";
import { readExpectation, readScope } from "./service";
import { statementAt } from "./statement";

export function createCompletionParser() {
  const parser = new PostgreSQL();
  const cache = new Map<string, { sql: string; result: ParsedCompletion | null }>();
  return (request: ParseRequest): ParsedCompletion | null => {
    const { document, version, sql, caret, wordRange } = request;
    const key = JSON.stringify([document, version, caret, wordRange]);
    const cached = cache.get(key);
    if (cached?.sql === sql) return cached.result;
    const statement = statementAt(sql, caret);
    const suggestion = parser.getSuggestionAtCaretPosition(statement.sql, statement.caret);
    const result = suggestion
      ? {
          expectation: readExpectation(
            suggestion.syntax as never,
            statement.caret,
            statement.localRange(wordRange),
          ),
          keywords: suggestion.keywords ?? [],
          scope: readScope(parser.getAllEntities(statement.sql, statement.caret) as never),
        }
      : null;
    if (result) result.expectation.range = statement.globalRange(result.expectation.range);
    if (cache.size >= 8) cache.delete(cache.keys().next().value as string);
    cache.set(key, { sql, result });
    return result;
  };
}

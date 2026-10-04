import type { CaretExpectation, CompletionRange, RelationRef } from "./types";

export interface ParseRequest {
  document: string;
  version: number;
  sql: string;
  caret: { lineNumber: number; column: number };
  wordRange: CompletionRange;
}

export interface ParsedCompletion {
  expectation: CaretExpectation;
  keywords: string[];
  scope: RelationRef[];
}

export interface ParseResponse {
  id: number;
  result?: ParsedCompletion | null;
  error?: string;
}

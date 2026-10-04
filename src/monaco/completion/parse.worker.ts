import { createCompletionParser } from "./parse";
import type { ParseRequest, ParseResponse } from "./parse-types";

const parse = createCompletionParser();
self.onmessage = (event: MessageEvent<ParseRequest & { id: number }>) => {
  const { id } = event.data;
  let response: ParseResponse;
  try {
    response = { id, result: parse(event.data) };
  } catch (error) {
    response = { id, error: String(error) };
  }
  self.postMessage(response);
};

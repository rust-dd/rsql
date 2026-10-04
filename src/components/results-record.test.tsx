import { renderToStaticMarkup } from "react-dom/server";
import { expect, it } from "vitest";
import { ResultsRecord } from "./results-record";

it("distinguishes NULL, literal null, and empty strings with duplicate aliases", () => {
  const html = renderToStaticMarkup(
    <ResultsRecord columns={["value", "value", "value"]} rows={[[null, "null", ""]]} />,
  );
  expect(html).toContain('title="SQL NULL"');
  expect(html).toContain(">null</td>");
  expect(html).toContain("empty string");
});

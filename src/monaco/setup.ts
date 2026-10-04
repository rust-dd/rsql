// SQL languages uses the moduleId/createData worker API removed in Monaco 0.55.
// Keep its tested editor version until a paired diagnostics-worker check passes.
import { loader } from "@monaco-editor/react";
import * as monaco from "monaco-editor";
import editorWorker from "monaco-editor/esm/vs/editor/editor.worker?worker";
import pgWorker from "monaco-sql-languages/esm/languages/pgsql/pgsql.worker?worker";

import "monaco-sql-languages/esm/languages/pgsql/pgsql.contribution";
import { LanguageIdEnum } from "monaco-sql-languages/esm/common/constants.js";
import { setupLanguageFeatures } from "monaco-sql-languages/esm/setupLanguageFeatures.js";
import { registerCompletion } from "./completion/provider";
import { registerTheme } from "./theme";

self.MonacoEnvironment = {
  getWorker(_workerId: string, label: string) {
    if (label === "pgsql") {
      return new pgWorker();
    }
    return new editorWorker();
  },
};

loader.config({ monaco });

setupLanguageFeatures(LanguageIdEnum.PG, {
  // Completion is registered directly in registerCompletion below; the library
  // still provides diagnostics and navigation through its worker.
  completionItems: false,
  diagnostics: true,
  definitions: true,
  references: true,
});

registerTheme(monaco);
registerCompletion(monaco);

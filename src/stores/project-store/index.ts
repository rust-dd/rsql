import { create } from "zustand";
import { immer } from "zustand/middleware/immer";
import { type ConnectionSlice, createConnectionSlice } from "./connection";
import { type CoreSlice, createCoreSlice } from "./core";

export { projectName } from "./core";

import { createIndexesSlice, type IndexesSlice } from "./indexes";
import { createSchemaSlice, type SchemaSlice } from "./schema";
import { createViewsSlice, type ViewsSlice } from "./views";

export type ProjectState = CoreSlice & ConnectionSlice & SchemaSlice & IndexesSlice & ViewsSlice;

export const useProjectStore = create<ProjectState>()(
  immer((...a) => ({
    ...createCoreSlice(...a),
    ...createConnectionSlice(...a),
    ...createSchemaSlice(...a),
    ...createIndexesSlice(...a),
    ...createViewsSlice(...a),
  })),
);

import { toast } from "sonner";
import type { StateCreator } from "zustand";
import { DriverFactory } from "@/lib/database-driver";
import { useSchemaIndexStore } from "@/stores/schema-index-store";
import { ProjectConnectionStatus as PCS } from "@/types";
import { projectName } from "./core";
import type { ProjectState } from "./index";

export type ConnectionSlice = {
  connect: (projectId: string) => Promise<void>;
  refreshConnection: (projectId: string) => Promise<void>;
};

export const createConnectionSlice: StateCreator<
  ProjectState,
  [["zustand/immer", never]],
  [],
  ConnectionSlice
> = (set, get) => ({
  connect: async (projectId: string) => {
    const { projects } = get();
    const d = projects[projectId];
    if (!d) return;

    set((s) => {
      s.status[projectId] = PCS.Connecting;
      s.connectionErrors[projectId] = "";
    });

    try {
      const driver = DriverFactory.getDriver(d.driver);
      const key: [string, string, string, string, string, string] = [
        d.username,
        d.password,
        d.database,
        d.host,
        d.port,
        d.ssl,
      ];
      const ssh =
        d.sshEnabled === "true"
          ? [d.sshHost, d.sshPort || "22", d.sshUser, d.sshPassword, d.sshKeyPath]
          : undefined;
      const st = await driver.connect(projectId, key, ssh);
      set((s) => {
        s.status[projectId] = st;
      });

      if (st === PCS.Connected) {
        const [sc, dbs, tsp] = await Promise.allSettled([
          driver.loadSchemas(projectId),
          driver.loadDatabases?.(projectId),
          driver.loadTablespaces?.(projectId),
        ]);
        const schemas = sc.status === "fulfilled" ? sc.value : [];
        set((s) => {
          s.schemas[projectId] = schemas;
          s.serverDatabases[projectId] = dbs.status === "fulfilled" && dbs.value ? dbs.value : [];
          s.serverTablespaces[projectId] = tsp.status === "fulfilled" && tsp.value ? tsp.value : [];
        });

        // Warm the schema the editor will almost certainly complete against, so
        // the first suggestion does not arrive a round trip late. Other schemas
        // load when something actually refers to them.
        const preferred = schemas.includes("public") ? "public" : schemas[0];
        if (preferred) {
          void useSchemaIndexStore.getState().ensureIndex(projectId, preferred);
        }
      }
    } catch (err: unknown) {
      const msg =
        err instanceof Error ? err.message : typeof err === "string" ? err : "Connection failed";
      set((s) => {
        s.status[projectId] = PCS.Failed;
        s.connectionErrors[projectId] = msg;
      });
      toast.error(`Connection failed: ${projectName(projects, projectId)}`, {
        description: msg,
        duration: 10000,
      });
    }
  },

  refreshConnection: async (projectId: string) => {
    const { projects, status, tables } = get();
    const d = projects[projectId];
    if (!d || status[projectId] !== PCS.Connected) return;

    // Remember which schemas had tables loaded so we can re-expand them after the wipe
    const schemaPrefix = `${projectId}::`;
    const expandedSchemas = Object.keys(tables)
      .filter((k) => k.startsWith(schemaPrefix))
      .map((k) => k.slice(schemaPrefix.length));

    set((s) => {
      for (const key of Object.keys(s.tables)) {
        if (key.startsWith(schemaPrefix)) delete s.tables[key];
      }
      for (const key of Object.keys(s.columns)) {
        if (key.startsWith(schemaPrefix)) delete s.columns[key];
      }
      for (const key of Object.keys(s.columnDetails)) {
        if (key.startsWith(schemaPrefix)) delete s.columnDetails[key];
      }
      for (const key of Object.keys(s.indexes)) {
        if (key.startsWith(schemaPrefix)) delete s.indexes[key];
      }
      for (const key of Object.keys(s.constraints)) {
        if (key.startsWith(schemaPrefix)) delete s.constraints[key];
      }
      for (const key of Object.keys(s.triggers)) {
        if (key.startsWith(schemaPrefix)) delete s.triggers[key];
      }
      for (const key of Object.keys(s.rules)) {
        if (key.startsWith(schemaPrefix)) delete s.rules[key];
      }
      for (const key of Object.keys(s.policies)) {
        if (key.startsWith(schemaPrefix)) delete s.policies[key];
      }
      for (const key of Object.keys(s.views)) {
        if (key.startsWith(schemaPrefix)) delete s.views[key];
      }
      for (const key of Object.keys(s.materializedViews)) {
        if (key.startsWith(schemaPrefix)) delete s.materializedViews[key];
      }
      for (const key of Object.keys(s.functions)) {
        if (key.startsWith(schemaPrefix)) delete s.functions[key];
      }
      for (const key of Object.keys(s.triggerFunctions)) {
        if (key.startsWith(schemaPrefix)) delete s.triggerFunctions[key];
      }

      delete s.schemas[projectId];
      delete s.serverDatabases[projectId];
      delete s.serverTablespaces[projectId];
    });

    try {
      const driver = DriverFactory.getDriver(d.driver);
      const [sc, dbs, tsp] = await Promise.allSettled([
        driver.loadSchemas(projectId),
        driver.loadDatabases?.(projectId),
        driver.loadTablespaces?.(projectId),
      ]);
      set((s) => {
        s.schemas[projectId] = sc.status === "fulfilled" ? sc.value : [];
        s.serverDatabases[projectId] = dbs.status === "fulfilled" && dbs.value ? dbs.value : [];
        s.serverTablespaces[projectId] = tsp.status === "fulfilled" && tsp.value ? tsp.value : [];
      });

      await Promise.all(
        expandedSchemas.map((schema) =>
          Promise.all([
            get().loadTables(projectId, schema),
            get().loadSchemaObjects(projectId, schema),
          ]),
        ),
      );

      toast.success("Connection refreshed");
    } catch {
      toast.error("Failed to refresh connection data");
    }
  },
});

import type { StateCreator } from "zustand";
import {
  deleteProject as deleteProjectApi,
  getProjects,
  insertProject,
  setServerAlias,
} from "@/tauri";
import type { DriverType, ProjectConnectionStatus, ProjectDetails, ProjectMap } from "@/types";
import { ProjectConnectionStatus as PCS } from "@/types";
import { useTabStore } from "../tab-store";
import type { ProjectState } from "./index";

export type CoreSlice = {
  projects: ProjectMap;
  status: Record<string, ProjectConnectionStatus>;
  connectionErrors: Record<string, string>;
  loadProjects: () => Promise<void>;
  deleteProject: (projectId: string) => Promise<void>;
  saveConnection: (projectId: string, details: ProjectDetails) => Promise<void>;
  updateConnection: (projectId: string, details: ProjectDetails) => Promise<void>;
  renameServer: (projectIds: string[], alias: string) => Promise<void>;
  addDatabaseToServer: (sourceProjectId: string, name: string, database: string) => Promise<void>;
};

export function parseProjectDetails(arr: string[]): ProjectDetails {
  return {
    driver: (arr[0] ?? "PGSQL") as DriverType,
    username: arr[1] ?? "",
    password: arr[2] ?? "",
    database: arr[3] ?? "",
    host: arr[4] ?? "",
    port: arr[5] ?? "",
    ssl: arr[6] ?? "false",
    sshEnabled: arr[7] ?? "false",
    sshHost: arr[8] ?? "",
    sshPort: arr[9] ?? "22",
    sshUser: arr[10] ?? "",
    sshPassword: arr[11] ?? "",
    sshKeyPath: arr[12] ?? "",
    name: arr[13] ?? "",
    serverAlias: arr[14] ?? "",
  };
}

/** Same order as the backend's columns; the legacy id is read-only and never sent back. */
export function toDetailArray(details: ProjectDetails): string[] {
  return [
    details.driver,
    details.username,
    details.password,
    details.database,
    details.host,
    details.port,
    details.ssl,
    details.sshEnabled ?? "false",
    details.sshHost ?? "",
    details.sshPort ?? "22",
    details.sshUser ?? "",
    details.sshPassword ?? "",
    details.sshKeyPath ?? "",
    details.name,
    details.serverAlias,
  ];
}

export function buildProjectMap(raw: Record<string, string[]>): {
  projects: ProjectMap;
  legacyIds: Map<string, string>;
} {
  const entries = Object.entries(raw).map(([id, arr]) => [id, parseProjectDetails(arr)] as const);
  entries.sort(([idA, a], [idB, b]) =>
    (a.name || idA).localeCompare(b.name || idB, undefined, { sensitivity: "base" }),
  );
  const legacyIds = new Map<string, string>();
  for (const [id, arr] of Object.entries(raw)) {
    if (arr[15]) legacyIds.set(arr[15], id);
  }
  return { projects: Object.fromEntries(entries), legacyIds };
}

export function projectName(projects: ProjectMap, projectId: string): string {
  return projects[projectId]?.name || projectId;
}

export const createCoreSlice: StateCreator<
  ProjectState,
  [["zustand/immer", never]],
  [],
  CoreSlice
> = (set, get) => ({
  projects: {},
  status: {},
  connectionErrors: {},

  loadProjects: async () => {
    try {
      const { projects, legacyIds } = buildProjectMap(await getProjects());
      set({ projects });
      useTabStore.getState().remapProjectIds(legacyIds);
    } catch (err) {
      console.error("Failed to load projects, retrying in 500ms...", err);
      // Retry once after a short delay — handles race where
      // the Tauri backend hasn't finished setup yet.
      await new Promise((r) => setTimeout(r, 500));
      try {
        const { projects, legacyIds } = buildProjectMap(await getProjects());
        set({ projects });
        useTabStore.getState().remapProjectIds(legacyIds);
      } catch (retryErr) {
        console.error("Failed to load projects after retry:", retryErr);
      }
    }
  },

  deleteProject: async (projectId: string) => {
    await deleteProjectApi(projectId);
    await get().loadProjects();
    set((s) => {
      s.status[projectId] = PCS.Disconnected;
    });
  },

  saveConnection: async (projectId: string, details: ProjectDetails) => {
    await insertProject(projectId, toDetailArray(details));
    await get().loadProjects();
  },

  updateConnection: async (projectId: string, details: ProjectDetails) => {
    await insertProject(projectId, toDetailArray(details));
    await get().loadProjects();
  },

  renameServer: async (projectIds: string[], alias: string) => {
    await setServerAlias(projectIds, alias.trim());
    await get().loadProjects();
  },

  addDatabaseToServer: async (sourceProjectId: string, name: string, database: string) => {
    const { projects } = get();
    const source = projects[sourceProjectId];
    if (!source) return;
    const details = { ...source, name, database };
    await get().saveConnection(crypto.randomUUID(), details);
  },
});

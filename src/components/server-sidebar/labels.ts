import { projectName } from "@/stores/project-store";
import type { ProjectMap } from "@/types";

export function serverGroupAlias(projects: ProjectMap, pids: string[]): string | undefined {
  return pids.map((pid) => projects[pid]?.serverAlias).find(Boolean);
}

export function serverGroupLabel(projects: ProjectMap, pids: string[]): string {
  const [first] = pids;
  if (!first) return "";
  const alias = serverGroupAlias(projects, pids);
  if (alias) return alias;
  if (pids.length === 1) return projectName(projects, first);
  const details = projects[first];
  return details ? `${details.host}:${details.port}` : first;
}

/** With an alias in the label, the address moves beside it. */
export function serverGroupAddress(projects: ProjectMap, pids: string[]): string | undefined {
  const details = pids[0] ? projects[pids[0]] : undefined;
  if (!details || !serverGroupAlias(projects, pids)) return undefined;
  return `${details.host}:${details.port}`;
}

/** Database rows carry the connection name unless the server row already shows it. */
export function databaseConnectionHint(
  projects: ProjectMap,
  pids: string[],
  pid: string,
  dbName: string,
): string | undefined {
  if (pids.length === 1 && !serverGroupAlias(projects, pids)) return undefined;
  const name = projectName(projects, pid);
  return name === dbName ? undefined : name;
}

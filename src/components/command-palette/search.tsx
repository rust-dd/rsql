import { Command } from "cmdk";
import { Activity, Database, Eye, FileCode, Layers, Network, Table } from "lucide-react";
import { projectName, type useProjectStore } from "@/stores/project-store";
import { useTabStore } from "@/stores/tab-store";

type ProjectState = ReturnType<typeof useProjectStore.getState>;

export function ConnectionsGroup({
  projects,
  status,
  schemas,
  onClose,
  connectProject,
}: {
  projects: ProjectState["projects"];
  status: ProjectState["status"];
  schemas: ProjectState["schemas"];
  onClose: () => void;
  connectProject: (id: string) => Promise<void> | void;
}) {
  if (Object.keys(projects).length === 0) return null;
  return (
    <Command.Group heading="Connections">
      {Object.entries(projects).map(([id, details]) => {
        const connected = status[id] === "Connected";
        return (
          <Command.Item
            key={`conn-${id}`}
            value={`${projectName(projects, id)} ${details.database} ${details.host} connection ${id}`}
            onSelect={() => {
              onClose();
              if (!connected) void connectProject(id);
            }}
          >
            <Database className="h-4 w-4 text-muted-foreground" />
            <span>{projectName(projects, id)}</span>
            <span className="cmdk-detail">
              {details.host}:{details.port}/{details.database}
            </span>
            <span className={`cmdk-meta ${connected ? "!bg-success/20 !text-success" : ""}`}>
              {connected ? "Connected" : "Connect"}
            </span>
          </Command.Item>
        );
      })}
      {Object.entries(projects).map(([id]) => {
        const connected = status[id] === "Connected";
        if (!connected) return null;
        return (
          <Command.Item
            key={`monitor-${id}`}
            value={`${projectName(projects, id)} performance monitor ${id}`}
            onSelect={() => {
              onClose();
              useTabStore.getState().openMonitorTab(id);
            }}
          >
            <Activity className="h-4 w-4 text-muted-foreground" />
            <span>Monitor {projectName(projects, id)}</span>
            <span className="cmdk-meta">Performance</span>
          </Command.Item>
        );
      })}
      {Object.entries(schemas).map(([projectId, projectSchemas]) =>
        projectSchemas.map((s) => (
          <Command.Item
            key={`erd-${projectId}-${s}`}
            value={`${projectName(projects, projectId)} ${s} ERD diagram ${projectId}`}
            onSelect={() => {
              onClose();
              useTabStore.getState().openERDTab(projectId, s);
            }}
          >
            <Network className="h-4 w-4 text-muted-foreground" />
            <span>
              ERD {projectName(projects, projectId)}/{s}
            </span>
            <span className="cmdk-meta">Diagram</span>
          </Command.Item>
        )),
      )}
    </Command.Group>
  );
}

export function DatabaseObjectsGroups({
  tables,
  views,
  materializedViews,
  functions,
  schemas,
  selectItem,
}: {
  tables: ProjectState["tables"];
  views: ProjectState["views"];
  materializedViews: ProjectState["materializedViews"];
  functions: ProjectState["functions"];
  schemas: ProjectState["schemas"];
  selectItem: (type: string, projectId: string, schema: string, name: string) => void;
}) {
  return (
    <>
      {Object.entries(tables).map(([key, schemaTables]) => {
        const [projectId, schema] = key.split("::");
        if (!schemaTables.length) return null;
        return (
          <Command.Group key={`t-${key}`} heading={`${projectId} / ${schema}`}>
            {schemaTables.map((t) => (
              <Command.Item
                key={`table-${key}-${t.name}`}
                value={`${schema} ${t.name} table`}
                onSelect={() => selectItem("table", projectId, schema, t.name)}
              >
                <Table className="h-4 w-4 text-muted-foreground" />
                <span>{t.name}</span>
                <span className="cmdk-meta">Table</span>
                {t.size && <span className="cmdk-detail">{t.size}</span>}
              </Command.Item>
            ))}
          </Command.Group>
        );
      })}

      {Object.entries(views).map(([key, schemaViews]) => {
        const [projectId, schema] = key.split("::");
        if (!schemaViews.length) return null;
        return (
          <Command.Group key={`v-${key}`} heading={`${projectId} / ${schema} Views`}>
            {schemaViews.map((v) => (
              <Command.Item
                key={`view-${key}-${v}`}
                value={`${schema} ${v} view`}
                onSelect={() => selectItem("view", projectId, schema, v)}
              >
                <Eye className="h-4 w-4 text-muted-foreground" />
                <span>{v}</span>
                <span className="cmdk-meta">View</span>
              </Command.Item>
            ))}
          </Command.Group>
        );
      })}

      {Object.entries(materializedViews).map(([key, matViews]) => {
        const [projectId, schema] = key.split("::");
        if (!matViews.length) return null;
        return (
          <Command.Group key={`mv-${key}`} heading={`${projectId} / ${schema} Mat. Views`}>
            {matViews.map((mv) => (
              <Command.Item
                key={`matview-${key}-${mv}`}
                value={`${schema} ${mv} materialized view`}
                onSelect={() => selectItem("matview", projectId, schema, mv)}
              >
                <Layers className="h-4 w-4 text-muted-foreground" />
                <span>{mv}</span>
                <span className="cmdk-meta">Mat. View</span>
              </Command.Item>
            ))}
          </Command.Group>
        );
      })}

      {Object.entries(functions).map(([key, fns]) => {
        const [projectId, schema] = key.split("::");
        if (!fns.length) return null;
        return (
          <Command.Group key={`fn-${key}`} heading={`${projectId} / ${schema} Functions`}>
            {fns.map((fn) => (
              <Command.Item
                key={`func-${key}-${fn.name}`}
                value={`${schema} ${fn.name} function`}
                onSelect={() => selectItem("function", projectId, schema, fn.name)}
              >
                <FileCode className="h-4 w-4 text-muted-foreground" />
                <span>{fn.name}</span>
                <span className="cmdk-meta">Function</span>
                <span className="cmdk-detail">
                  ({fn.arguments || ""}) → {fn.returnType}
                </span>
              </Command.Item>
            ))}
          </Command.Group>
        );
      })}

      {Object.entries(schemas).map(([projectId, projectSchemas]) => {
        if (!projectSchemas.length) return null;
        return (
          <Command.Group key={`s-${projectId}`} heading={`${projectId} Schemas`}>
            {projectSchemas.map((s) => (
              <Command.Item
                key={`schema-${projectId}-${s}`}
                value={`${s} schema`}
                onSelect={() => selectItem("schema", projectId, s, s)}
              >
                <Database className="h-4 w-4 text-muted-foreground" />
                <span>{s}</span>
                <span className="cmdk-meta">Schema</span>
              </Command.Item>
            ))}
          </Command.Group>
        );
      })}
    </>
  );
}

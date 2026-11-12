import {
  Activity,
  Columns3,
  Copy,
  Database,
  Edit3,
  HardDrive,
  Link2,
  List,
  Loader2,
  Package,
  Pencil,
  Plus,
  RefreshCw,
  Server,
  Settings,
  Shield,
  Trash2,
  Zap,
} from "lucide-react";
import { cn } from "@/lib/utils";
import type { ProjectDetails } from "@/types";
import { ProjectConnectionStatus } from "@/types";
import { DEPTH, LEAF_ICON, SECTION_ICON } from "./constants";
import { databaseConnectionHint, serverGroupAddress, serverGroupLabel } from "./labels";
import { renderSchemas } from "./render-schema-objects";
import { SectionHeader } from "./section-header";
import { StatusDot, TreeRow } from "./tree-row";
import type { SidebarRenderCtx } from "./types";

/**
 * Render a single server-fingerprint group with its databases, roles
 * and tablespaces. Auto-grouping by host:port:user:ssh fingerprint.
 */
export function renderServerGroup(ctx: SidebarRenderCtx, fp: string, pids: string[]) {
  const {
    projects,
    status,
    serverDatabases,
    serverTablespaces,
    isOpen,
    toggle,
    onConnect,
    addDatabaseToServer,
    deleteProject,
    refreshConnection,
    openTab,
    openMonitorTab,
    openNotifyTab,
    openRolesTab,
    openSchemaDiffTab,
    openExtensionsTab,
    openEnumsTab,
    openPgSettingsTab,
    setAddDbSource,
    setRenameServerPids,
    showMenu,
    copy,
    onEditConnection,
  } = ctx;

  const primaryDetails: ProjectDetails | undefined = projects[pids[0]];
  if (!primaryDetails) return null;

  const gKey = `srv::${fp}`;
  const dbCatKey = `${gKey}::databases`;

  const serverLabel = serverGroupLabel(projects, pids);
  const serverAddress = serverGroupAddress(projects, pids);
  const connectedPid = pids.find((p) => status[p] === ProjectConnectionStatus.Connected);

  // Union of databases discovered from pg_database + ones we have a project for
  const discoveredDbs = new Set<string>();
  for (const pid of pids) {
    const dbs = serverDatabases[pid];
    if (dbs)
      dbs.forEach((db) => {
        discoveredDbs.add(db);
      });
    const d = projects[pid];
    if (d?.database) discoveredDbs.add(d.database);
  }
  const dbToProject = new Map<string, string>();
  for (const pid of pids) {
    const d = projects[pid];
    if (d?.database) dbToProject.set(d.database, pid);
  }
  const allDbs = Array.from(discoveredDbs).sort();

  const anyConnected = pids.some((p) => status[p] === ProjectConnectionStatus.Connected);
  const anyConnecting = pids.some((p) => status[p] === ProjectConnectionStatus.Connecting);

  return (
    <div key={gKey}>
      <TreeRow
        depth={DEPTH.server}
        icon={<Server className="h-3.5 w-3.5 text-primary" />}
        label={serverLabel}
        bold
        expanded={isOpen(gKey, true)}
        onClick={() => toggle(gKey)}
        onContextMenu={(e) =>
          showMenu(e, [
            { header: "Server" },
            ...(connectedPid
              ? [
                  {
                    label: "New Query",
                    icon: <Plus className="h-3 w-3" />,
                    onClick: () => openTab(connectedPid),
                  },
                  {
                    label: "Performance Monitor",
                    icon: <Activity className="h-3 w-3" />,
                    onClick: () => openMonitorTab(connectedPid),
                  },
                  {
                    label: "PG Settings",
                    icon: <Settings className="h-3 w-3" />,
                    onClick: () => openPgSettingsTab(connectedPid),
                  },
                ]
              : []),
            {
              label: "Add Database",
              icon: <Plus className="h-3 w-3" />,
              onClick: () => setAddDbSource(pids[0]),
            },
            {
              label: "Rename Server",
              icon: <Pencil className="h-3 w-3" />,
              onClick: () => setRenameServerPids(pids),
            },
            ...(onEditConnection && pids.length === 1
              ? [
                  {
                    label: "Edit Connection",
                    icon: <Edit3 className="h-3 w-3" />,
                    onClick: () => onEditConnection(pids[0]),
                  },
                ]
              : []),
            { separator: true as const },
            {
              label: "Copy Name",
              icon: <Copy className="h-3 w-3" />,
              onClick: () => copy(serverLabel),
            },
            { separator: true as const },
            {
              label: "Delete",
              icon: <Trash2 className="h-3 w-3" />,
              onClick: () => {
                for (const pid of pids) void deleteProject(pid);
              },
              destructive: true,
            },
          ])
        }
        trailing={
          <span className="flex min-w-0 items-center gap-1.5">
            {serverAddress && (
              <span
                className="truncate font-mono text-[10px] text-muted-foreground"
                title={serverAddress}
              >
                {serverAddress}
              </span>
            )}
            <StatusDot
              className={cn(
                anyConnected && "bg-success shadow-[0_0_6px_currentColor]",
                anyConnecting && "bg-warning shadow-[0_0_6px_currentColor]",
                !anyConnected && !anyConnecting && "bg-muted-foreground/30",
              )}
            />
          </span>
        }
      />

      {isOpen(gKey, true) && (
        <>
          <SectionHeader
            depth={DEPTH.cat}
            icon={<Database className={SECTION_ICON} />}
            label="Databases"
            count={allDbs.length > 0 ? allDbs.length : undefined}
            expanded={isOpen(dbCatKey, true)}
            onClick={() => toggle(dbCatKey)}
          />

          {isOpen(dbCatKey, true) &&
            allDbs.map((dbName) => {
              const dbPid = dbToProject.get(dbName);
              const dbKey = `db::${fp}::${dbName}`;

              if (dbPid) {
                const dbConn = status[dbPid];
                const isDbConnected = dbConn === ProjectConnectionStatus.Connected;
                const isDbConnecting = dbConn === ProjectConnectionStatus.Connecting;
                const isDbFailed = dbConn === ProjectConnectionStatus.Failed;
                const connectionHint = databaseConnectionHint(projects, pids, dbPid, dbName);
                return (
                  <div key={dbName}>
                    <TreeRow
                      depth={DEPTH.db}
                      icon={
                        isDbConnecting ? (
                          <Loader2 className="h-3.5 w-3.5 animate-spin text-muted-foreground" />
                        ) : (
                          <Database className={SECTION_ICON} />
                        )
                      }
                      label={dbName}
                      expanded={isDbConnected ? isOpen(dbKey, true) : undefined}
                      onClick={() => {
                        if (!isDbConnected && !isDbConnecting) void onConnect(dbPid);
                        else if (isDbConnected) toggle(dbKey);
                      }}
                      onContextMenu={(e) =>
                        showMenu(e, [
                          { header: "Database" },
                          {
                            label: "New Query",
                            icon: <Plus className="h-3 w-3" />,
                            onClick: () => openTab(dbPid),
                          },
                          {
                            label: isDbConnected ? "Reconnect" : "Connect",
                            icon: <RefreshCw className="h-3 w-3" />,
                            onClick: () => void onConnect(dbPid),
                          },
                          ...(isDbConnected
                            ? [
                                {
                                  label: "Refresh",
                                  icon: <RefreshCw className="h-3 w-3" />,
                                  onClick: () => void refreshConnection(dbPid),
                                },
                                {
                                  label: "LISTEN/NOTIFY",
                                  icon: <Zap className="h-3 w-3" />,
                                  onClick: () => openNotifyTab(dbPid),
                                },
                                {
                                  label: "Schema Diff",
                                  icon: <Columns3 className="h-3 w-3" />,
                                  onClick: () => openSchemaDiffTab(dbPid),
                                },
                                {
                                  label: "Extensions",
                                  icon: <Package className="h-3 w-3" />,
                                  onClick: () => openExtensionsTab(dbPid),
                                },
                                {
                                  label: "Enum Types",
                                  icon: <List className="h-3 w-3" />,
                                  onClick: () => openEnumsTab(dbPid),
                                },
                              ]
                            : []),
                          ...(onEditConnection
                            ? [
                                { separator: true as const },
                                {
                                  label: "Edit Connection",
                                  icon: <Edit3 className="h-3 w-3" />,
                                  onClick: () => onEditConnection(dbPid),
                                },
                              ]
                            : []),
                          { separator: true as const },
                          {
                            label: "Copy Name",
                            icon: <Copy className="h-3 w-3" />,
                            onClick: () => copy(dbName),
                          },
                          { separator: true as const },
                          {
                            label: "Delete",
                            icon: <Trash2 className="h-3 w-3" />,
                            onClick: () => void deleteProject(dbPid),
                            destructive: true,
                          },
                        ])
                      }
                      trailing={
                        <span className="flex min-w-0 items-center gap-1.5">
                          {connectionHint && (
                            <span
                              className="truncate font-mono text-[10px] text-muted-foreground"
                              title={connectionHint}
                            >
                              {connectionHint}
                            </span>
                          )}
                          <StatusDot
                            className={cn(
                              isDbConnected && "bg-success shadow-[0_0_6px_currentColor]",
                              isDbConnecting && "bg-warning shadow-[0_0_6px_currentColor]",
                              isDbFailed && "bg-destructive shadow-[0_0_6px_currentColor]",
                              !dbConn && "bg-muted-foreground/30",
                            )}
                          />
                        </span>
                      }
                    />

                    {isDbConnected && isOpen(dbKey, true) && renderSchemas(ctx, dbPid)}
                  </div>
                );
              } else {
                // No project entry yet — clicking auto-creates one
                return (
                  <TreeRow
                    key={dbName}
                    depth={DEPTH.db}
                    icon={<Database className="h-3.5 w-3.5 text-muted-foreground/50" />}
                    muted
                    label={dbName}
                    onClick={() => void addDatabaseToServer(pids[0], dbName, dbName)}
                    onContextMenu={(e) =>
                      showMenu(e, [
                        {
                          label: "Connect",
                          icon: <Link2 className="h-3 w-3" />,
                          onClick: () => void addDatabaseToServer(pids[0], dbName, dbName),
                        },
                        {
                          label: "Copy Name",
                          icon: <Copy className="h-3 w-3" />,
                          onClick: () => copy(dbName),
                        },
                      ])
                    }
                  />
                );
              }
            })}

          <TreeRow
            depth={DEPTH.cat}
            icon={<Shield className={SECTION_ICON} />}
            label="Login/Group Roles"
            muted
            onClick={() => {
              if (connectedPid) {
                openRolesTab(connectedPid);
              } else {
                void onConnect(pids[0]).then(() => {
                  const p = pids.find((id) => status[id] === ProjectConnectionStatus.Connected);
                  if (p) openRolesTab(p);
                });
              }
            }}
          />

          {(() => {
            const tspCatKey = `${gKey}::tablespaces`;
            const tspData = connectedPid ? serverTablespaces[connectedPid] || [] : [];
            return (
              <>
                <SectionHeader
                  depth={DEPTH.cat}
                  icon={<HardDrive className={SECTION_ICON} />}
                  label="Tablespaces"
                  count={tspData.length > 0 ? tspData.length : undefined}
                  expanded={isOpen(tspCatKey)}
                  onClick={() => {
                    if (connectedPid) {
                      toggle(tspCatKey);
                    } else {
                      void onConnect(pids[0]);
                    }
                  }}
                />
                {isOpen(tspCatKey) &&
                  tspData.map(([name, owner, location]) => (
                    <TreeRow
                      key={name}
                      depth={DEPTH.db}
                      icon={<HardDrive className={LEAF_ICON} />}
                      label={name}
                      detail={location ? `${owner} ${location}` : owner}
                      onContextMenu={(e) =>
                        showMenu(e, [
                          {
                            label: "Copy Name",
                            icon: <Copy className="h-3 w-3" />,
                            onClick: () => copy(name),
                          },
                        ])
                      }
                    />
                  ))}
              </>
            );
          })()}
        </>
      )}
    </div>
  );
}

import {
  Check,
  Columns3,
  Copy,
  Database,
  Eye,
  FileCode,
  Key,
  Layers,
  Link2,
  Loader2,
  Pencil,
  Table,
  Zap,
} from "lucide-react";
import { DialogDescription, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { cn } from "@/lib/utils";
import { projectName, useProjectStore } from "@/stores/project-store";
import type { ObjectType, Tab } from "./types";

const objectIcon: Record<ObjectType, React.ReactNode> = {
  table: <Table className="h-4 w-4 text-primary" />,
  view: <Eye className="h-4 w-4 text-blue-500" />,
  matview: <Layers className="h-4 w-4 text-purple-500" />,
  function: <FileCode className="h-4 w-4 text-amber-500" />,
  "trigger-function": <Zap className="h-4 w-4 text-orange-500" />,
};

const objectLabel: Record<ObjectType, string> = {
  table: "Table",
  view: "View",
  matview: "Materialized View",
  function: "Function",
  "trigger-function": "Trigger Function",
};

export const typeColor: Record<ObjectType, string> = {
  table: "from-primary/20 to-primary/5",
  view: "from-blue-500/20 to-blue-500/5",
  matview: "from-purple-500/20 to-purple-500/5",
  function: "from-amber-500/20 to-amber-500/5",
  "trigger-function": "from-orange-500/20 to-orange-500/5",
};

const tabIcons: Partial<Record<Tab, React.ReactNode>> = {
  overview: <Database className="h-3 w-3" />,
  columns: <Columns3 className="h-3 w-3" />,
  indexes: <Key className="h-3 w-3" />,
  fkeys: <Link2 className="h-3 w-3" />,
  structure: <Pencil className="h-3 w-3" />,
  ddl: <FileCode className="h-3 w-3" />,
  actions: <Zap className="h-3 w-3" />,
};

export function ModalHeader({
  objectType,
  schema,
  name,
  projectId,
  loading,
  copied,
  copyText,
  availableTabs,
  activeTab,
  setActiveTab,
}: {
  objectType: ObjectType;
  schema: string;
  name: string;
  projectId: string;
  loading: boolean;
  copied: string | null;
  copyText: (text: string, label: string) => void;
  availableTabs: { key: Tab; label: string }[];
  activeTab: Tab;
  setActiveTab: (tab: Tab) => void;
}) {
  const connectionName = useProjectStore((s) => projectName(s.projects, projectId));
  return (
    <div className={cn("relative px-5 pt-5 pb-3 bg-gradient-to-b", typeColor[objectType])}>
      <div className="absolute inset-0 bg-[radial-gradient(ellipse_at_top,rgba(255,255,255,0.03),transparent_70%)]" />
      <DialogHeader className="relative">
        <div className="flex items-center gap-2.5">
          <div className="flex items-center justify-center h-8 w-8 rounded-lg bg-background/60 backdrop-blur-sm border border-border/30">
            {objectIcon[objectType]}
          </div>
          <div className="min-w-0 flex-1">
            <DialogTitle className="flex items-center gap-2 text-base">
              <span className="truncate">{name}</span>
              <button
                type="button"
                onClick={() => copyText(`"${schema}"."${name}"`, "name")}
                className="text-muted-foreground/40 hover:text-foreground transition-colors shrink-0"
                title="Copy qualified name"
              >
                {copied === "name" ? (
                  <Check className="h-3 w-3 text-success" />
                ) : (
                  <Copy className="h-3 w-3" />
                )}
              </button>
            </DialogTitle>
            <DialogDescription className="flex items-center gap-1.5 mt-0.5">
              <span className="inline-flex items-center gap-1 px-1.5 py-0.5 rounded bg-background/40 text-[10px] font-medium uppercase tracking-wider">
                {objectLabel[objectType]}
              </span>
              <span className="font-mono text-[11px]">{schema}</span>
              <span className="text-muted-foreground/30">|</span>
              <span className="font-mono text-[11px] text-muted-foreground/60">
                {connectionName}
              </span>
              {loading && <Loader2 className="h-3 w-3 animate-spin ml-1" />}
            </DialogDescription>
          </div>
        </div>
      </DialogHeader>

      {/* Tab switcher - pill style */}
      <div className="relative flex gap-0.5 mt-3 bg-background/30 backdrop-blur-sm rounded-lg p-0.5 border border-border/20">
        {availableTabs.map((tab) => (
          <button
            key={tab.key}
            type="button"
            onClick={() => setActiveTab(tab.key)}
            className={cn(
              "flex items-center gap-1.5 px-3 py-1.5 text-[11px] font-medium whitespace-nowrap rounded-md transition-all",
              activeTab === tab.key
                ? "bg-background text-foreground shadow-sm shadow-black/10"
                : "text-muted-foreground hover:text-foreground",
            )}
          >
            {tabIcons[tab.key]}
            {tab.label}
          </button>
        ))}
      </div>
    </div>
  );
}

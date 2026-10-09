"use client";

import {
  Activity,
  Bot,
  Database,
  FlaskConical,
  House,
  ScanSearch,
  Settings,
  Sparkles,
  type LucideIcon,
} from "lucide-react";
import { cn } from "../../lib/cva.config";
import { TabsList, TabsTrigger } from "../ui/tabs";
import { useWorkerConnected } from "./hooks/useWorkerConnected";
import type { InvestigationActivity } from "./model/status";
import type { LensList } from "./model/types";
import type { LensTab } from "./route";

interface LensTabsProps {
  readonly activity: InvestigationActivity;
  readonly workers: LensList["workers"] | null;
  readonly onNavigate: (tab: LensTab) => void;
  readonly orientation: "horizontal" | "vertical";
  readonly collapsed?: boolean;
}

const NAVIGATION_ITEMS = [
  { value: "home", label: "Home", icon: House },
  { value: "agents", label: "Agents", icon: Bot },
  { value: "traces", label: "Traces", icon: Activity },
  { value: "findings", label: "Findings", icon: Sparkles },
  { value: "investigations", label: "Investigations", icon: ScanSearch },
  { value: "datasets", label: "Datasets", icon: Database },
  { value: "evals", label: "Evals", icon: FlaskConical },
] as const;

export function LensTabs({ activity, workers, onNavigate, orientation, collapsed = false }: LensTabsProps) {
  const connected = useWorkerConnected(workers);
  const vertical = orientation === "vertical";
  const item = (value: LensTab, label: string, Icon: LucideIcon, status?: string) => (
    <TabsTrigger
      key={value}
      value={value}
      title={status ?? (collapsed ? label : undefined)}
      onClick={() => onNavigate(value)}
      aria-description={
        value === "investigations" && activity !== "idle" ? `An investigation is ${activity}` : undefined
      }
      className={cn(
        "flex-none gap-2.5 text-[13px] font-medium text-muted-foreground data-active:text-foreground data-active:shadow-none",
        vertical
          ? "lens-nav-tab h-9 w-full justify-start rounded-md px-2.5 after:hidden data-active:bg-sidebar-accent data-active:text-sidebar-accent-foreground hover:bg-sidebar-accent/60"
          : "h-11 rounded-none px-2 after:inset-x-0 after:bottom-0 after:h-0.5",
        collapsed && "justify-center px-0",
      )}
    >
      <Icon aria-hidden="true" className="size-4" />
      <span className={cn(collapsed && "sr-only")}>{label}</span>
      {value === "investigations" && activity !== "idle" && (
        <span
          aria-hidden="true"
          className={cn(
            "size-1.5 shrink-0 rounded-full bg-info motion-safe:animate-pulse",
            collapsed ? "absolute top-1.5 right-1.5" : "ml-auto",
          )}
        />
      )}
      {value === "settings" && !connected && (
        <span
          aria-hidden="true"
          className={cn(
            "size-1.5 shrink-0 rounded-full bg-warning",
            collapsed ? "absolute top-1.5 right-1.5" : "ml-auto",
          )}
        />
      )}
    </TabsTrigger>
  );
  return (
    <TabsList
      aria-label="Lens"
      activateOnFocus={false}
      variant={vertical ? "default" : "line"}
      className={cn(
        "flex rounded-none bg-transparent p-0",
        vertical
          ? "h-auto! min-h-fit w-full flex-1 flex-col items-stretch justify-start gap-1"
          : "h-11 w-max justify-start gap-3",
      )}
    >
      {NAVIGATION_ITEMS.map(({ value, label, icon }) => item(value, label, icon))}
      {workers && item("settings", "Settings", Settings, connected ? "Analysis configured" : "Configure analysis")}
    </TabsList>
  );
}

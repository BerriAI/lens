"use client";

import { useState } from "react";
import { useTheme } from "next-themes";
import { useMediaQuery } from "usehooks-ts";
import {
  Activity,
  ArrowUpRight,
  BookOpen,
  Bot,
  Database,
  Moon,
  PanelLeftClose,
  PanelLeftOpen,
  ScanSearch,
  Settings,
  Sparkles,
  Sun,
  type LucideIcon,
} from "lucide-react";
import { Button } from "../ui/button";
import { TabsList, TabsTrigger } from "../ui/tabs";
import { Sheet, SheetContent, SheetDescription, SheetTitle, SheetTrigger } from "../ui/sheet";
import { cn } from "../../lib/cva.config";
import { AgentPicker } from "./agents/AgentPicker";
import type { LensAgents } from "./agents/AgentScoped";
import { LensBrand } from "./LensBrand";
import { useWorkerConnected } from "./hooks/useWorkerConnected";
import type { InvestigationActivity } from "./model/status";
import type { LensList } from "./model/types";
import type { LensTab } from "./route";

interface LensSidebarProps {
  readonly agents: LensAgents;
  readonly activity: InvestigationActivity;
  readonly workers: LensList["workers"] | null;
  readonly onNavigate: (tab: LensTab) => void;
}

const NAVIGATION_ITEMS = [
  { value: "agents", label: "Agents", icon: Bot },
  { value: "traces", label: "Traces", icon: Activity },
  { value: "findings", label: "Findings", icon: Sparkles },
  { value: "investigations", label: "Investigations", icon: ScanSearch },
  { value: "datasets", label: "Datasets", icon: Database },
] as const;

export function LensSidebar(props: LensSidebarProps) {
  const [collapsed, setCollapsed] = useState(false);
  const [mobileOpen, setMobileOpen] = useState(false);
  const mobile = useMediaQuery("(max-width: 767px)", {
    initializeWithValue: false,
  });
  const navigate = (tab: LensTab) => {
    props.onNavigate(tab);
    setMobileOpen(false);
  };
  const navigation = (compact: boolean) => (
    <SidebarNavigation
      {...props}
      collapsed={compact}
      onNavigate={navigate}
      onSelectAgent={(agent) => {
        props.agents.select(agent);
        setMobileOpen(false);
      }}
    />
  );
  if (mobile)
    return (
      <Sheet open={mobileOpen} onOpenChange={setMobileOpen}>
        <SheetTrigger
          render={<Button variant="ghost" size="icon-sm" className="absolute top-3 left-3 z-raised" />}
          aria-label="Open navigation"
        >
          <PanelLeftOpen aria-hidden="true" className="size-4" />
        </SheetTrigger>
        <SheetContent side="left" className="w-72! gap-0 overflow-y-auto bg-sidebar p-3" aria-describedby={undefined}>
          <SheetTitle className="sr-only">Lens navigation</SheetTitle>
          <SheetDescription className="sr-only">Browse agents and your Lens workspace</SheetDescription>
          <Button
            variant="ghost"
            className="mb-5 h-12 w-fit px-2"
            aria-label="Lens agents"
            onClick={() => navigate("agents")}
          >
            <LensBrand />
          </Button>
          {navigation(false)}
        </SheetContent>
      </Sheet>
    );
  return (
    <aside
      aria-label="Lens navigation"
      className={cn(
        "flex shrink-0 flex-col overflow-y-auto border-r border-sidebar-border bg-sidebar px-3 py-4 text-sidebar-foreground transition-[width] duration-200 motion-reduce:transition-none",
        collapsed ? "w-16 px-2" : "w-60",
      )}
    >
      <div className={cn("mb-6 flex h-9 items-center gap-1", collapsed ? "justify-center" : "justify-between px-1")}>
        {!collapsed && (
          <Button
            variant="ghost"
            className="h-auto min-w-0 p-0 hover:bg-transparent"
            aria-label="Lens agents"
            onClick={() => navigate("agents")}
          >
            <LensBrand />
          </Button>
        )}
        <Button
          variant="ghost"
          size="icon-sm"
          className="text-muted-foreground"
          aria-label={collapsed ? "Expand navigation" : "Collapse navigation"}
          title={collapsed ? "Expand navigation" : "Collapse navigation"}
          onClick={() => setCollapsed(!collapsed)}
        >
          {collapsed ? <PanelLeftOpen aria-hidden="true" /> : <PanelLeftClose aria-hidden="true" />}
        </Button>
      </div>
      {navigation(collapsed)}
    </aside>
  );
}

function SidebarNavigation({
  agents,
  activity,
  workers,
  collapsed,
  onNavigate,
  onSelectAgent,
}: LensSidebarProps & {
  readonly collapsed: boolean;
  readonly onSelectAgent: (agent: string) => void;
}) {
  const connected = useWorkerConnected(workers);
  const item = (value: LensTab, label: string, Icon: LucideIcon, status?: string) => (
    <TabsTrigger
      value={value}
      title={status ?? (collapsed ? label : undefined)}
      onClick={() => onNavigate(value)}
      aria-description={
        value === "investigations" && activity !== "idle" ? `An investigation is ${activity}` : undefined
      }
      className={cn(
        "h-9 w-full flex-none justify-start gap-2.5 rounded-md px-2.5 text-[13px] font-medium text-muted-foreground after:hidden data-active:bg-sidebar-accent data-active:text-sidebar-accent-foreground data-active:shadow-none hover:bg-sidebar-accent/60",
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
    <>
      {!collapsed && agents.agent && (
        <div className="mb-4 min-w-0 [&>button]:h-10 [&>button]:w-full [&>button]:max-w-none [&>button]:justify-start [&>button]:border-0 [&>button]:bg-muted/50 [&>button]:px-2.5 [&>button]:text-[13px] [&>button>svg:last-child]:ml-auto">
          <AgentPicker agent={agents.agent} agents={agents.list.agents} onSelect={onSelectAgent} />
        </div>
      )}
      <TabsList
        aria-label="Lens"
        activateOnFocus={false}
        className="flex h-auto! min-h-fit w-full flex-1 flex-col items-stretch justify-start gap-1 rounded-none bg-transparent p-0"
      >
        {NAVIGATION_ITEMS.map(({ value, label, icon: Icon }) => (
          <div key={value}>{item(value, label, Icon)}</div>
        ))}
        {workers && item("settings", "Settings", Settings, connected ? "Worker connected" : "Connect worker")}
      </TabsList>
      <div className="mt-4 border-t border-sidebar-border pt-3">
        <ThemeToggle collapsed={collapsed} />
      </div>
      <a
        href="https://docs.litellm.ai/docs/proxy/lens"
        target="_blank"
        rel="noopener noreferrer"
        title={collapsed ? "Documentation" : undefined}
        className={cn(
          "mt-1 flex h-9 shrink-0 items-center gap-2.5 rounded-md px-2.5 text-[13px] text-muted-foreground outline-none hover:bg-sidebar-accent hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring",
          collapsed && "justify-center px-0",
        )}
      >
        <BookOpen aria-hidden="true" className="size-4" />
        <span className={cn(collapsed && "sr-only")}>Documentation</span>
        {!collapsed && <ArrowUpRight aria-hidden="true" className="ml-auto size-3.5" />}
      </a>
    </>
  );
}

function ThemeToggle({ collapsed }: { readonly collapsed: boolean }) {
  const { resolvedTheme, setTheme } = useTheme();
  return (
    <Button
      variant="ghost"
      aria-label="Toggle color theme"
      title="Toggle color theme"
      onClick={() => setTheme(resolvedTheme === "dark" ? "light" : "dark")}
      className={cn(
        "h-9 w-full justify-start gap-2.5 px-2.5 text-[13px] font-normal text-muted-foreground hover:bg-sidebar-accent",
        collapsed && "justify-center px-0",
      )}
    >
      <Moon aria-hidden="true" className="size-4 dark:hidden" />
      <Sun aria-hidden="true" className="hidden size-4 dark:block" />
      <span className={cn("dark:hidden", collapsed && "sr-only")}>Dark mode</span>
      <span className={cn("hidden dark:inline", collapsed && "sr-only")}>Light mode</span>
    </Button>
  );
}

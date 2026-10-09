"use client";

import { useState } from "react";
import { STANDALONE_DOCS_URL, useLensHost } from "../../host/LensHost";
import { useTheme } from "next-themes";
import { useIsClient, useMediaQuery } from "usehooks-ts";
import { ArrowUpRight, BookOpen, Moon, PanelLeftClose, PanelLeftOpen, Sun } from "lucide-react";
import { Button } from "../ui/button";
import { Sheet, SheetContent, SheetDescription, SheetTitle, SheetTrigger } from "../ui/sheet";
import { cn } from "../../lib/cva.config";
import { AgentPicker } from "./agents/AgentPicker";
import type { LensAgents } from "./agents/AgentScoped";
import { LensBrand } from "./LensBrand";
import { LensTabs } from "./LensTabs";
import type { InvestigationActivity } from "./model/status";
import type { LensList } from "./model/types";
import type { LensTab } from "./route";

interface LensSidebarProps {
  readonly agents: LensAgents;
  readonly activity: InvestigationActivity;
  readonly workers: LensList["workers"] | null;
  readonly onNavigate: (tab: LensTab) => void;
}

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
        <SheetContent
          side="left"
          className="lens-shell-overlay lens-sidebar w-72! gap-0 overflow-y-auto bg-sidebar p-3"
          aria-describedby={undefined}
        >
          <SheetTitle className="sr-only">Lens navigation</SheetTitle>
          <SheetDescription className="sr-only">Browse agents and your Lens workspace</SheetDescription>
          <Button
            variant="ghost"
            className="mb-5 h-12 w-fit px-2 hover:bg-transparent dark:hover:bg-transparent"
            aria-label="Lens home"
            onClick={() => navigate("home")}
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
        "lens-sidebar flex shrink-0 flex-col overflow-y-auto border-r border-sidebar-border bg-sidebar px-3 py-4 text-sidebar-foreground transition-[width] duration-200 motion-reduce:transition-none",
        collapsed ? "w-16 px-2" : "w-60",
      )}
    >
      <div className={cn("mb-6 flex h-9 items-center gap-1", collapsed ? "justify-center" : "justify-between px-1")}>
        {!collapsed && (
          <Button
            variant="ghost"
            className="h-auto min-w-0 p-0 hover:bg-transparent dark:hover:bg-transparent"
            aria-label="Lens home"
            onClick={() => navigate("home")}
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
  const standalone = useLensHost().surface === "standalone";
  return (
    <>
      {!collapsed && agents.agent && (
        <div className="mb-4 min-w-0 [&>button]:h-10 [&>button]:w-full [&>button]:max-w-none [&>button]:justify-start [&>button]:border-0 [&>button]:bg-muted/50 [&>button]:px-2.5 [&>button]:text-[13px] [&>button>svg:last-child]:ml-auto">
          <AgentPicker agent={agents.agent} agents={agents.list.agents} onSelect={onSelectAgent} />
        </div>
      )}
      {!collapsed && (
        <p className="mb-3 px-2.5 font-mono text-[10px] font-medium tracking-[0.16em] text-muted-foreground uppercase">
          Workspace
        </p>
      )}
      <LensTabs
        orientation="vertical"
        activity={activity}
        workers={workers}
        collapsed={collapsed}
        onNavigate={onNavigate}
      />
      <div className="mt-4 border-t border-sidebar-border pt-3">
        <ThemeToggle collapsed={collapsed} />
      </div>
      <a
        href={standalone ? STANDALONE_DOCS_URL : "https://docs.litellm.ai/docs/proxy/lens"}
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
  const mounted = useIsClient();
  if (collapsed)
    return (
      <Button
        variant="ghost"
        aria-label="Toggle color theme"
        title="Toggle color theme"
        onClick={() => setTheme(resolvedTheme === "dark" ? "light" : "dark")}
        className="h-9 w-full px-0 text-muted-foreground hover:bg-sidebar-accent"
      >
        <Moon aria-hidden="true" className="size-4 dark:hidden" />
        <Sun aria-hidden="true" className="hidden size-4 dark:block" />
      </Button>
    );
  return (
    <div
      role="group"
      aria-label="Color theme"
      className="flex gap-1 rounded-lg border border-sidebar-border bg-muted/50 p-1"
    >
      <Button
        variant="ghost"
        size="sm"
        aria-label="Light mode"
        aria-pressed={mounted ? resolvedTheme === "light" : undefined}
        onClick={() => setTheme("light")}
        className="flex-1 gap-2 bg-background text-xs text-foreground shadow-xs hover:bg-background dark:bg-transparent dark:text-muted-foreground dark:shadow-none dark:hover:bg-sidebar-accent"
      >
        <Sun aria-hidden="true" className="size-3.5" />
        Light
      </Button>
      <Button
        variant="ghost"
        size="sm"
        aria-label="Dark mode"
        aria-pressed={mounted ? resolvedTheme === "dark" : undefined}
        onClick={() => setTheme("dark")}
        className="flex-1 gap-2 text-xs text-muted-foreground hover:bg-sidebar-accent dark:bg-background dark:text-foreground dark:shadow-xs dark:hover:bg-background"
      >
        <Moon aria-hidden="true" className="size-3.5" />
        Dark
      </Button>
    </div>
  );
}

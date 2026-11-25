"use client";

import { useState, type ReactNode } from "react";
import {
  STANDALONE_DOCS_URL,
  useLensHost,
  type LensHost,
} from "../../host/LensHost";
import { useTheme } from "next-themes";
import { useIsClient, useMediaQuery } from "usehooks-ts";
import {
  ArrowUpRight,
  BookOpen,
  CalendarDays,
  ChevronsUpDown,
  LogOut,
  Moon,
  PanelLeftClose,
  PanelLeftOpen,
  Sun,
} from "lucide-react";
import { Button } from "../ui/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "../ui/dropdown-menu";
import {
  Sheet,
  SheetContent,
  SheetDescription,
  SheetTitle,
  SheetTrigger,
} from "../ui/sheet";
import {
  Tooltip,
  TooltipContent,
  TooltipProvider,
  TooltipTrigger,
} from "../ui/tooltip";
import { cn } from "../../lib/cva.config";
import { LensBrand } from "./LensBrand";
import type { WorkspaceTab } from "./workspace/route";

interface LensSidebarProps {
  readonly onNavigate: (tab: WorkspaceTab) => void;
  readonly children: (collapsed: boolean, close: () => void) => ReactNode;
}

export function LensSidebar(props: LensSidebarProps) {
  const host = useLensHost();
  const [collapsed, setCollapsed] = useState(false);
  const [mobileOpen, setMobileOpen] = useState(false);
  const mobile = useMediaQuery("(max-width: 767px)", {
    initializeWithValue: false,
  });
  const navigate = (tab: WorkspaceTab) => {
    props.onNavigate(tab);
    setMobileOpen(false);
  };
  const navigation = (compact: boolean) => (
    <SidebarNavigation collapsed={compact}>
      {props.children(compact, () => {
        setMobileOpen(false);
        host.navigation?.onNavigate();
      })}
    </SidebarNavigation>
  );
  if (host.navigation) {
    const Container = host.navigation.Container;
    return (
      <Container>
        <nav
          aria-label="Lens navigation"
          className="lens-shell-overlay flex min-h-0 flex-1 flex-col px-3 py-2 text-sidebar-foreground"
        >
          {navigation(host.navigation.collapsed)}
        </nav>
      </Container>
    );
  }
  if (mobile)
    return (
      <Sheet open={mobileOpen} onOpenChange={setMobileOpen}>
        <SheetTrigger
          render={
            <Button
              variant="ghost"
              size="icon-sm"
              className="absolute top-3 left-3 z-raised"
            />
          }
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
          <SheetDescription className="sr-only">
            Browse agents and your Lens workspace
          </SheetDescription>
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
      <div
        className={cn(
          "mb-6 flex h-9 items-center gap-1",
          collapsed ? "justify-center" : "justify-between px-1",
        )}
      >
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
          {collapsed ? (
            <PanelLeftOpen aria-hidden="true" />
          ) : (
            <PanelLeftClose aria-hidden="true" />
          )}
        </Button>
      </div>
      {navigation(collapsed)}
    </aside>
  );
}

function SidebarNavigation({
  collapsed,
  children,
}: {
  readonly collapsed: boolean;
  readonly children: ReactNode;
}) {
  const host = useLensHost();
  const standalone = host.surface === "standalone";
  return (
    <>
      <div className="flex min-h-0 flex-1 flex-col">{children}</div>
      {!host.navigation && (
        <div className="mt-4 border-t border-sidebar-border pt-3">
          <ThemeToggle collapsed={collapsed} />
        </div>
      )}
      <a
        href={
          standalone
            ? STANDALONE_DOCS_URL
            : "https://docs.litellm.ai/docs/proxy/lens"
        }
        onClick={host.navigation?.onNavigate}
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
        {!collapsed && (
          <ArrowUpRight aria-hidden="true" className="ml-auto size-3.5" />
        )}
      </a>
      <TooltipProvider>
        <Tooltip>
          <TooltipTrigger
            render={
              <a
                href="https://calendly.com/d/dtr8-nnr-3yy/litellm-product-team-feedback"
                onClick={host.navigation?.onNavigate}
                target="_blank"
                rel="noopener noreferrer"
                className={cn(
                  "mt-1 flex h-9 shrink-0 items-center gap-2.5 rounded-md px-2.5 text-[13px] text-muted-foreground outline-none hover:bg-sidebar-accent hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring",
                  collapsed && "justify-center px-0",
                )}
              />
            }
          >
            <CalendarDays aria-hidden="true" className="size-4" />
            <span className={cn(collapsed && "sr-only")}>Book a call</span>
            {!collapsed && (
              <ArrowUpRight aria-hidden="true" className="ml-auto size-3.5" />
            )}
          </TooltipTrigger>
          <TooltipContent side="right">
            Book a call with LiteLLM founders
          </TooltipContent>
        </Tooltip>
      </TooltipProvider>
      {!host.navigation && host.user && (
        <UserMenu user={host.user} collapsed={collapsed} />
      )}
    </>
  );
}

const roleLabel = (role: string) =>
  role
    .replace(/^proxy_/, "")
    .split("_")
    .map((word) => word.charAt(0).toUpperCase() + word.slice(1))
    .join(" ");

function UserMenu({
  user,
  collapsed,
}: {
  readonly user: NonNullable<LensHost["user"]>;
  readonly collapsed: boolean;
}) {
  const role = roleLabel(user.role);
  return (
    <div className="mt-3 border-t border-sidebar-border pt-3">
      <DropdownMenu>
        <DropdownMenuTrigger
          render={
            <Button
              variant="ghost"
              aria-label={`Account: ${user.name}, ${role}`}
              title={collapsed ? user.name : undefined}
              className={cn(
                "h-auto w-full justify-start gap-2.5 bg-muted/50 px-2 py-1.5 text-left font-normal hover:bg-sidebar-accent",
                collapsed && "justify-center bg-transparent px-0",
              )}
            />
          }
        >
          <span
            aria-hidden="true"
            className="flex size-7 shrink-0 items-center justify-center rounded-full bg-primary text-[11px] font-medium text-primary-foreground"
          >
            {user.name.slice(0, 2).toUpperCase()}
          </span>
          {!collapsed && (
            <>
              <span className="flex min-w-0 flex-col">
                <span className="truncate text-[13px] text-foreground">
                  {user.name}
                </span>
                <span className="truncate text-xs text-muted-foreground">
                  {role}
                </span>
              </span>
              <ChevronsUpDown
                aria-hidden="true"
                className="ml-auto size-3.5 shrink-0 text-muted-foreground"
              />
            </>
          )}
        </DropdownMenuTrigger>
        <DropdownMenuContent side="top" className={cn(collapsed && "w-40")}>
          <DropdownMenuItem onClick={user.signOut}>
            <LogOut />
            Logout
          </DropdownMenuItem>
        </DropdownMenuContent>
      </DropdownMenu>
    </div>
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

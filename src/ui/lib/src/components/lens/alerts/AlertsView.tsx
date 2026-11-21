"use client";

import { useQuery } from "@tanstack/react-query";
import { Bell, Settings2 } from "lucide-react";
import { Button } from "../../ui/button";
import { Skeleton } from "../../ui/skeleton";
import { Sheet, SheetContent, SheetDescription, SheetHeader, SheetTitle, SheetTrigger } from "../../ui/sheet";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "../../ui/tabs";
import { useLensApi } from "../data/LensServices";
import { lensQueries } from "../data/queries";
import { useAlertsRoute } from "../route";
import { SignalSettings } from "../settings/signals/SignalSettings";
import type { OpenTraceRouting } from "../traces/routing";
import { LensPageHeader } from "../ui/LensPageHeader";
import { AlertDashboard } from "./AlertDashboard";
import { AlertRuleSettings } from "./AlertRuleSettings";

export function AlertsView({
  accessToken,
  canConfigure,
  onOpenTrace,
  onOpenSignal,
}: {
  accessToken: string;
  canConfigure: boolean;
  onOpenTrace: OpenTraceRouting["openTrace"];
  onOpenSignal: OpenTraceRouting["openSignal"];
}) {
  const api = useLensApi();
  const route = useAlertsRoute();
  const overview = useQuery({
    ...lensQueries.signalOverview(api),
    refetchInterval: 30_000,
  });
  const rules = useQuery({
    queryKey: ["alert-rule-overview", api.scope],
    queryFn: () => api.alertRuleOverview(),
    refetchInterval: 30_000,
  });
  return (
    <Sheet
      open={canConfigure && route.settings !== null}
      onOpenChange={(open) => !open && route.setSettings(null)}
    >
      <div aria-label="Alerts" role="region" className="flex min-h-0 w-full min-w-0 flex-1 flex-col bg-card">
        <LensPageHeader
          page="alerts"
          placement="workspace"
          title="Alerts"
          actions={
            canConfigure && (
              <>
                <SheetTrigger
                  render={<Button variant="outline" size="sm" className="text-xs" />}
                  onClick={() => route.setSettings("slack")}
                >
                  <Bell aria-hidden="true" /> Slack notifications
                </SheetTrigger>
                <SheetTrigger
                  render={<Button size="sm" className="text-xs" />}
                  onClick={() => route.setSettings("signals")}
                >
                  <Settings2 aria-hidden="true" /> Configure alerts
                </SheetTrigger>
              </>
            )
          }
        />
        {overview.error || rules.error ? (
          <div role="alert" className="lens-page-body space-y-3 text-sm">
            <p className="font-medium">Could not load alert activity</p>
            <p className="text-muted-foreground">{overview.error?.message ?? rules.error?.message}</p>
            <Button
              variant="outline"
              size="sm"
              onClick={() => {
                void overview.refetch();
                void rules.refetch();
              }}
            >
              Try again
            </Button>
          </div>
        ) : overview.data && rules.data ? (
          <AlertDashboard
            accessToken={accessToken}
            data={overview.data}
            rules={rules.data}
            onOpenTrace={onOpenTrace}
            onOpenSignal={onOpenSignal}
            refreshing={overview.isFetching || rules.isFetching}
            onRefresh={() => {
              void overview.refetch();
              void rules.refetch();
            }}
          />
        ) : (
          <div className="lens-page-body">
            <Skeleton aria-label="Loading alert activity" className="h-40 w-full" />
          </div>
        )}
      </div>
      {canConfigure && (
        <SheetContent className="lens-shell-overlay w-full! gap-0 sm:max-w-2xl!">
          <SheetHeader className="shrink-0 border-b px-6 py-5 pr-14">
            <SheetTitle>{route.settings === "slack" ? "Slack notifications" : "Configure alerts"}</SheetTitle>
            <SheetDescription>
              {route.settings === "slack"
                ? "Send rule alerts and selected model signals to a Slack channel."
                : "Choose what Lens checks in new traces across all agents and teams."}
            </SheetDescription>
          </SheetHeader>
          {route.settings === "signals" || route.settings === "rules" ? (
            <Tabs
              value={route.settings}
              onValueChange={(value) => route.setSettings(value === "rules" ? "rules" : "signals")}
              className="min-h-0 flex-1 gap-0"
            >
              <div className="shrink-0 border-b px-6">
                <TabsList variant="line" aria-label="Alert configuration" className="h-11 gap-6 px-0">
                  <TabsTrigger value="signals" className="flex-none px-0">
                    Model signals
                  </TabsTrigger>
                  <TabsTrigger value="rules" className="flex-none px-0">
                    Rules
                  </TabsTrigger>
                </TabsList>
              </div>
              <TabsContent value="signals" keepMounted className="min-h-0 overflow-y-auto px-6 py-6">
                <SignalSettings section="signals" onConfigureSignals={() => route.setSettings("signals")} />
              </TabsContent>
              <TabsContent value="rules" keepMounted className="min-h-0 overflow-y-auto px-6 py-6">
                <AlertRuleSettings />
              </TabsContent>
            </Tabs>
          ) : route.settings ? (
            <div className="min-h-0 flex-1 overflow-y-auto px-6 py-6">
              <SignalSettings
                section={route.settings}
                onConfigureSignals={() => route.setSettings("signals")}
              />
            </div>
          ) : null}
        </SheetContent>
      )}
    </Sheet>
  );
}

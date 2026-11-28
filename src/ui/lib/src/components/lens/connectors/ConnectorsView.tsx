"use client";

import { useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { ArrowLeft, ArrowUpRight, Plug } from "lucide-react";
import { useLensHost } from "../../../host/LensHost";
import { getRequestBaseUrl } from "../../../lib/http/runtime";
import type { McpCreateRequest, McpOverview } from "../../../lib/http/worker";
import { Button } from "../../ui/button";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "../../ui/tabs";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "../../ui/dialog";
import { useLensAccessToken } from "../data/LensServices";
import { LensPageHeader } from "../ui/LensPageHeader";
import { useConnectorRoute, type ConnectorSection } from "../route";
import { connectorOverview, createConnection, revokeConnection } from "./api";
import {
  connectionEndpoint,
  connectionStatus,
  MCP_DOCS_URL,
  TOOLS,
} from "./model";
import { McpSetup, SELECT_CLASS } from "./McpSetup";
import { McpAccess, McpUsage } from "./McpActivity";

export function ConnectorsView({
  demo,
  onExitDemo,
}: {
  demo: boolean;
  onExitDemo: () => void;
}) {
  const accessToken = useLensAccessToken();
  return demo ? (
    <div className="lens-page-body space-y-4">
      <h2 className="text-lg font-semibold">Connectors</h2>
      <p className="text-sm text-muted-foreground">
        Connectors use your deployment’s credentials and activity. Leave demo
        data to set up Lens MCP
      </p>
      <Button onClick={onExitDemo}>Use my deployment</Button>
    </div>
  ) : (
    <LiveConnectors key={accessToken} />
  );
}

function LiveConnectors() {
  const accessToken = useLensAccessToken();
  const host = useLensHost();
  const route = useConnectorRoute();
  const [creation, setCreation] = useState<{
    id: string;
    token: string | null;
    overview: McpOverview;
  } | null>(null);
  const [pending, setPending] = useState(false);
  const [formVersion, setFormVersion] = useState(0);
  const [error, setError] = useState<string | null>(null);
  const [watchUntil, setWatchUntil] = useState(0);
  const [revoke, setRevoke] = useState<string | null>(null);
  const overview = useQuery({
    queryKey: [
      "lens-connectors",
      accessToken,
      route.hours,
      route.connection,
      route.cursor,
      route.toMs,
    ],
    queryFn: ({ signal }) =>
      connectorOverview(
        accessToken,
        route.hours,
        route.connection,
        route.cursor,
        route.toMs ?? undefined,
        signal,
      ),
    retry: false,
    gcTime: 0,
    refetchInterval: Date.now() < watchUntil ? 3000 : false,
  });
  const createdSetup =
    route.section === "setup" && creation?.id === route.connection
      ? creation
      : null;
  const data = overview.data ?? createdSetup?.overview;
  const selected = data?.connections.find(
    (connection) => connection.id === route.connection,
  );
  const usage = data?.activity.usage.find(
    (row) => row.connection_id === route.connection,
  );
  const status = selected ? connectionStatus(selected, usage) : "";
  const watching = Date.now() < watchUntil && status !== "Connected";
  const endpoint =
    selected && data && typeof window !== "undefined"
      ? connectionEndpoint({
          origin: window.location.origin,
          pathname: window.location.pathname,
          base: getRequestBaseUrl(),
          embedded: host.surface === "embedded",
          authentication: selected.authentication,
          gatewayUrl: data.gateway_url,
          id: selected.id,
        })
      : null;
  const create = async (request: McpCreateRequest) => {
    if (!data) return;
    setPending(true);
    setError(null);
    try {
      const created = await createConnection(accessToken, request);
      setCreation({
        id: created.connection.id,
        token: created.token ?? null,
        overview: {
          ...data,
          connections: [
            ...data.connections.filter(
              (item) => item.id !== created.connection.id,
            ),
            created.connection,
          ],
        },
      });
      route.setup(created.connection.id);
      setWatchUntil(Date.now() + 120_000);
    } catch {
      setError(
        "Could not finish creating the connection. Refresh Access before retrying. If it was created but the token was not received, revoke it and create a new connection",
      );
      await overview.refetch();
    } finally {
      setPending(false);
    }
  };
  const confirmRevoke = async () => {
    if (!revoke) return;
    setPending(true);
    setError(null);
    try {
      await revokeConnection(accessToken, revoke);
      if (creation?.id === revoke) setCreation(null);
      setRevoke(null);
      await overview.refetch();
    } catch {
      setError("Could not revoke the connection. Refresh and try again");
    } finally {
      setPending(false);
    }
  };
  const newConnection = () => {
    setFormVersion((version) => version + 1);
    setCreation(null);
    route.setup(null);
    setError(null);
  };
  return (
    <div className="w-full">
      <LensPageHeader
        page="connectors"
        title="Connectors"
        description="Connect your agents to Lens and see how they use it"
        actions={
          <Button
            size="sm"
            variant="outline"
            onClick={() => void overview.refetch()}
            disabled={overview.isFetching}
          >
            {overview.isFetching ? "Refreshing…" : "Refresh"}
          </Button>
        }
      />
      <div className="lens-page-body mx-auto w-full max-w-7xl">
        {!route.detail ? (
          <button
            type="button"
            onClick={route.open}
            className="flex w-full max-w-3xl items-center gap-4 rounded-lg border p-5 text-left transition-colors hover:bg-muted/40 focus-visible:outline-ring"
          >
            <Plug
              aria-hidden="true"
              className="size-5 shrink-0 text-muted-foreground"
            />
            <span className="flex-1">
              <span className="block font-semibold">Lens MCP</span>
              <span className="mt-1 block text-sm text-muted-foreground">
                Let your agent inspect traces, failures, and cost
              </span>
              <span className="mt-3 block text-xs text-muted-foreground">
                {data
                  ? `${data.connections.length} named connections · 5 read-only tools`
                  : "Setup, usage, access, and agent instructions"}
              </span>
            </span>
            <ArrowUpRight aria-hidden="true" className="size-4" />
          </button>
        ) : (
          <>
            <Button
              variant="ghost"
              size="sm"
              className="mb-4 -ml-2"
              onClick={route.close}
            >
              <ArrowLeft aria-hidden="true" className="size-4" />
              Connectors
            </Button>
            <div className="mb-5 flex flex-wrap items-start justify-between gap-4">
              <div>
                <h2 className="text-xl font-semibold">Lens MCP</h2>
                <p className="mt-1 text-sm text-muted-foreground">
                  {data?.authentication === "gateway"
                    ? "Connected through your LiteLLM gateway"
                    : "Read-only tools for your coding agent"}
                </p>
              </div>
              <div className="flex flex-wrap gap-2">
                {data?.can_create &&
                  (route.section !== "setup" || selected || error) && (
                    <Button size="sm" onClick={newConnection}>
                      New connection
                    </Button>
                  )}
                <a
                  href={MCP_DOCS_URL}
                  target="_blank"
                  rel="noopener noreferrer"
                  className="inline-flex items-center gap-1 text-sm underline underline-offset-4"
                >
                  Docs
                  <ArrowUpRight aria-hidden="true" className="size-3" />
                </a>
              </div>
            </div>
            {(error || overview.isError) && (
              <div
                role="alert"
                className="mb-4 rounded-lg border border-destructive/30 p-4 text-sm"
              >
                {error ??
                  "Could not load connections. Check your sign-in and refresh to retry"}
              </div>
            )}
            {overview.isPending && !data && (
              <p role="status" className="py-6 text-sm text-muted-foreground">
                Loading connections…
              </p>
            )}
            {data && (
              <Tabs
                value={route.section}
                onValueChange={(value) =>
                  route.sectionTo(value as ConnectorSection)
                }
              >
                <div className="flex flex-wrap items-center justify-between gap-3 border-b pb-2">
                  <TabsList variant="line" aria-label="Lens MCP">
                    <TabsTrigger value="setup">Setup</TabsTrigger>
                    <TabsTrigger value="usage">Usage</TabsTrigger>
                    <TabsTrigger value="access">Access</TabsTrigger>
                    <TabsTrigger value="tools">Tools</TabsTrigger>
                  </TabsList>
                  <label className="flex items-center gap-2 text-xs text-muted-foreground">
                    Activity window
                    <select
                      aria-label="Activity window"
                      className={SELECT_CLASS}
                      value={route.hours}
                      onChange={(event) =>
                        route.window(Number(event.target.value))
                      }
                    >
                      <option value={24}>Last 24 hours</option>
                      <option value={168}>Last 7 days</option>
                      <option value={720}>Last 30 days</option>
                    </select>
                  </label>
                </div>
                <TabsContent value="setup">
                  {route.connection && !selected ? (
                    <div className="space-y-3 py-6">
                      <p className="text-sm">
                        This connection is unavailable to your account
                      </p>
                      <Button onClick={newConnection}>
                        Set up a connection
                      </Button>
                    </div>
                  ) : (
                    <McpSetup
                      key={selected?.id ?? `new-${formVersion}`}
                      overview={data}
                      selected={selected}
                      endpoint={endpoint}
                      secret={
                        creation && selected?.id === creation.id
                          ? creation.token
                          : null
                      }
                      onCreate={create}
                      pending={pending}
                      watching={watching}
                      onWatch={() => {
                        setWatchUntil(Date.now() + 120_000);
                        void overview.refetch();
                      }}
                      status={status}
                    />
                  )}
                </TabsContent>
                <TabsContent value="usage">
                  <McpUsage
                    overview={data}
                    selected={route.connection}
                    onSelect={route.activity}
                    onNext={() =>
                      route.next(data.activity.next_cursor ?? null, data.to_ms)
                    }
                    onFirst={route.cursor ? route.first : undefined}
                  />
                </TabsContent>
                <TabsContent value="access">
                  <McpAccess
                    overview={data}
                    onOpen={route.setup}
                    onRevoke={setRevoke}
                    pending={pending}
                  />
                </TabsContent>
                <TabsContent value="tools">
                  <div className="max-w-4xl space-y-5 py-6">
                    <p className="text-sm text-muted-foreground">
                      MCP (Model Context Protocol) lets your agent call these
                      Lens tools. They can read traces but cannot modify them
                    </p>
                    <dl className="divide-y">
                      {TOOLS.map(([name, description]) => (
                        <div
                          key={name}
                          className="grid gap-1 py-4 sm:grid-cols-[14rem_1fr]"
                        >
                          <dt className="font-mono text-sm">{name}</dt>
                          <dd className="text-sm text-muted-foreground">
                            {description}
                          </dd>
                        </div>
                      ))}
                    </dl>
                    <h3 className="font-semibold">Connection basics</h3>
                    <p className="text-sm text-muted-foreground">
                      Use Streamable HTTP with bearer authentication. Lens
                      accepts JSON-RPC POST requests; opening the endpoint in a
                      browser returns 405. OAuth and persistent server sessions
                      are not supported
                    </p>
                    <h3 className="font-semibold">Troubleshooting</h3>
                    <dl className="space-y-3 text-sm">
                      <div>
                        <dt className="font-medium">
                          401, expired, or revoked
                        </dt>
                        <dd className="text-muted-foreground">
                          Check Access and your environment variable. Replace an
                          expired or lost credential with a new connection
                        </dd>
                      </div>
                      <div>
                        <dt className="font-medium">
                          403 or permission denied
                        </dt>
                        <dd className="text-muted-foreground">
                          Use the connection owner’s LiteLLM key and check their
                          current trace access
                        </dd>
                      </div>
                      <div>
                        <dt className="font-medium">No traces found</dt>
                        <dd className="text-muted-foreground">
                          Send traces from your project, then check the agent
                          name and time range. A tracing key sends traces; it
                          cannot read through MCP
                        </dd>
                      </div>
                      <div>
                        <dt className="font-medium">Cannot reach Lens</dt>
                        <dd className="text-muted-foreground">
                          The agent must be able to reach the displayed URL. On
                          a remote agent, localhost refers to that agent’s
                          machine
                        </dd>
                      </div>
                    </dl>
                  </div>
                </TabsContent>
              </Tabs>
            )}
          </>
        )}
      </div>
      <Dialog
        open={revoke !== null}
        onOpenChange={(open) => {
          if (!open && !pending) setRevoke(null);
        }}
      >
        <DialogContent>
          <DialogHeader>
            <DialogTitle>Revoke this connection?</DialogTitle>
            <DialogDescription>
              Requests through this connection will be denied. You can create a
              new connection later. Gateway keys retain their other permissions
            </DialogDescription>
          </DialogHeader>
          {error && (
            <p role="alert" className="text-sm">
              {error}
            </p>
          )}
          <DialogFooter>
            <Button
              variant="outline"
              disabled={pending}
              onClick={() => setRevoke(null)}
            >
              Cancel
            </Button>
            <Button
              variant="destructive"
              disabled={pending}
              onClick={() => void confirmRevoke()}
            >
              {pending ? "Revoking…" : "Revoke connection"}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </div>
  );
}

import { Button } from "../../ui/button";
import type { McpOverview } from "../../../lib/http/worker";
import { connectionStatus } from "./model";

const TABLE =
  "w-full text-left text-sm [&_th]:whitespace-nowrap [&_th]:px-3 [&_th]:py-3 [&_th]:font-medium [&_th]:text-muted-foreground [&_td]:px-3 [&_td]:py-3 [&_tr]:border-b";
export const dateTime = (value: string | number) =>
  new Date(value).toLocaleString();

export function McpUsage({
  overview,
  selected,
  onSelect,
  onNext,
  onFirst,
}: {
  overview: McpOverview;
  selected: string | null;
  onSelect: (id: string | null) => void;
  onNext: () => void;
  onFirst?: () => void;
}) {
  const activity = overview.activity;
  return (
    <div className="space-y-5 py-6">
      <p className="text-sm text-muted-foreground">
        {overview.can_manage_all ? "All connections" : "Your connections"}. Tool
        calls from {dateTime(overview.from_ms)} to {dateTime(overview.to_ms)} (
        {Intl.DateTimeFormat().resolvedOptions().timeZone}). Activity is
        retained for {overview.retention_days} days
      </p>
      <div className="overflow-x-auto">
        <table className={TABLE}>
          <caption className="sr-only">MCP tool usage by connection</caption>
          <thead>
            <tr>
              <th>Connection</th>
              <th>Connection owner</th>
              <th>Client</th>
              <th>Calls</th>
              <th>Errors</th>
              <th>Last call</th>
            </tr>
          </thead>
          <tbody>
            {activity.usage.map((row) => (
              <tr key={row.connection_id}>
                <td>
                  <Button
                    variant="link"
                    className="h-auto max-w-64 justify-start truncate p-0"
                    onClick={() => onSelect(row.connection_id)}
                  >
                    {row.connection_name}
                  </Button>
                </td>
                <td className="max-w-64 break-words">{row.owner_name}</td>
                <td>{row.client}</td>
                <td>{row.calls.toLocaleString()}</td>
                <td>{row.errors.toLocaleString()}</td>
                <td className="whitespace-nowrap">
                  {dateTime(row.last_used_ms)}
                </td>
              </tr>
            ))}
          </tbody>
        </table>
        {!activity.usage.length && (
          <p className="py-6 text-sm text-muted-foreground">
            No tool calls in this time window. Run the starter prompt from your
            connected agent
          </p>
        )}
        {activity.usage_truncated && (
          <p className="py-2 text-sm">
            Showing the 1,000 most recently used connections. Choose a shorter
            time window to narrow the results
          </p>
        )}
      </div>
      <div className="flex flex-wrap items-center justify-between gap-2">
        <h3 className="font-semibold">
          {selected ? "Connection activity" : "Recent activity"}
        </h3>
        {selected && (
          <Button size="sm" variant="outline" onClick={() => onSelect(null)}>
            Show all activity
          </Button>
        )}
      </div>
      <p className="text-sm text-muted-foreground">
        Metadata only: no prompts, tool arguments, results, or credentials.
        Denied attempts belong to the targeted connection; they do not identify
        who sent them. Client names are setup labels
      </p>
      <div className="overflow-x-auto">
        <table className={TABLE}>
          <caption className="sr-only">MCP request log</caption>
          <thead>
            <tr>
              <th>Time</th>
              <th>Connection</th>
              <th>Tool or method</th>
              <th>Outcome</th>
              <th>Latency</th>
            </tr>
          </thead>
          <tbody>
            {activity.calls.map((call) => (
              <tr key={call.id}>
                <td className="whitespace-nowrap">{dateTime(call.at_ms)}</td>
                <td className="max-w-64 break-words">{call.connection_name}</td>
                <td>
                  <code>{call.tool ?? call.method}</code>
                </td>
                <td>{call.outcome.replaceAll("_", " ")}</td>
                <td>{call.latency_ms} ms</td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
      {!activity.calls.length && (
        <p className="text-sm text-muted-foreground">
          No requests recorded in this window
        </p>
      )}
      <div className="flex gap-2">
        {onFirst && (
          <Button variant="outline" onClick={onFirst}>
            Latest requests
          </Button>
        )}
        {activity.next_cursor && (
          <Button variant="outline" onClick={onNext}>
            Older requests
          </Button>
        )}
      </div>
    </div>
  );
}

export function McpAccess({
  overview,
  onOpen,
  onRevoke,
  pending,
}: {
  overview: McpOverview;
  onOpen: (id: string) => void;
  onRevoke: (id: string) => void;
  pending: boolean;
}) {
  return (
    <div className="space-y-4 py-6">
      <p className="text-sm text-muted-foreground">
        Each connection can expire or be revoked independently. Revoking a
        gateway connection blocks its named URL; it does not revoke the LiteLLM
        key or its access through the original /lens/mcp endpoint
      </p>
      <div className="overflow-x-auto">
        <table className={TABLE}>
          <caption className="sr-only">MCP connection access</caption>
          <thead>
            <tr>
              <th>Connection</th>
              <th>Owner</th>
              <th>Scope</th>
              <th>Expires</th>
              <th>Status</th>
              <th>
                <span className="sr-only">Actions</span>
              </th>
            </tr>
          </thead>
          <tbody>
            {overview.connections.map((connection) => (
              <tr key={connection.id}>
                <td>
                  <Button
                    variant="link"
                    className="h-auto max-w-64 justify-start truncate p-0"
                    onClick={() => onOpen(connection.id)}
                  >
                    {connection.name}
                  </Button>
                  <div className="mt-1 text-xs text-muted-foreground">
                    {connection.client}
                  </div>
                </td>
                <td className="max-w-64 break-words">
                  {connection.owner_name}
                  {connection.authentication === "token" && (
                    <div className="text-xs text-muted-foreground">
                      Unverified label
                    </div>
                  )}
                </td>
                <td>
                  {connection.authentication === "token"
                    ? "All workspace traces, read-only"
                    : "Owner’s current gateway permissions"}
                </td>
                <td className="whitespace-nowrap">
                  {dateTime(connection.expires_at)}
                </td>
                <td>
                  {connectionStatus(
                    connection,
                    overview.activity.usage.find(
                      (row) => row.connection_id === connection.id,
                    ),
                  )}
                </td>
                <td>
                  {overview.can_create && !connection.revoked_at && (
                    <Button
                      size="sm"
                      variant="outline"
                      disabled={pending}
                      onClick={() => onRevoke(connection.id)}
                    >
                      Revoke
                    </Button>
                  )}
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
      {!overview.connections.length && (
        <p className="text-sm text-muted-foreground">
          No named connections yet. Create one in Setup
        </p>
      )}
      <p className="text-sm text-muted-foreground">
        Existing credentials appear in Usage after their next request. Shared
        credentials cannot identify individual users; create separate
        connections for attribution
      </p>
    </div>
  );
}

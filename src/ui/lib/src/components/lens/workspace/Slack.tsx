import { useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Copy, Loader2, Slack } from "lucide-react";
import { Button } from "../../ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "../../ui/dialog";
import { copyToClipboard } from "../../../utils/dataUtils";
import { SlackSetup } from "./SlackSetup";
import type { WorkspaceApi } from "./Chat";

export function SlackConnection({
  api,
  scope,
  chatId,
  compact = false,
}: {
  api: WorkspaceApi;
  scope: string;
  chatId?: string;
  compact?: boolean;
}) {
  const [open, setOpen] = useState(false);
  const [editing, setEditing] = useState(false);
  const client = useQueryClient();
  const statusKey = ["agent-workspace", scope, "slack-status"];
  const status = useQuery({
    queryKey: statusKey,
    queryFn: api.slackStatus,
    enabled: open,
    refetchInterval: open ? 2000 : false,
  });
  const link = useQuery({
    queryKey: ["agent-workspace", scope, "slack", chatId],
    queryFn: () => api.slackLink(chatId!),
    enabled: open && !!chatId && status.data?.connected === true,
    refetchInterval: open && !!chatId && status.data?.connected ? 2000 : false,
  });
  const code = useMutation({ mutationFn: () => api.slackCode(chatId!) });
  const unlink = useMutation({
    mutationFn: () => api.unlinkSlack(chatId!),
    onSuccess: () => {
      code.reset();
      void link.refetch();
    },
  });
  const copy = useMutation({
    mutationFn: () =>
      copyToClipboard(code.data!.command, "Slack command copied"),
  });
  return (
    <Dialog
      open={open}
      onOpenChange={(value) => {
        setOpen(value);
        if (!value) {
          code.reset();
          setEditing(false);
        }
      }}
    >
      <Button
        variant={chatId ? "outline" : "ghost"}
        size="sm"
        className={
          chatId
            ? undefined
            : "w-full justify-start gap-2.5 text-muted-foreground"
        }
        aria-label={chatId ? "Slack" : "Connect Slack"}
        title={compact ? "Connect Slack" : undefined}
        onClick={() => setOpen(true)}
      >
        {!chatId && <Slack className="size-4 shrink-0" />}
        <span className={compact ? "sr-only" : undefined}>
          {chatId ? "Slack" : "Connect Slack"}
        </span>
      </Button>
      <DialogContent className="sm:max-w-lg">
        <DialogHeader>
          <DialogTitle>
            {chatId && status.data?.configured && !editing
              ? "Continue in Slack"
              : "Connect Slack"}
          </DialogTitle>
          <DialogDescription>
            {chatId && status.data?.configured && !editing
              ? "Link a Slack thread to this conversation. Replies and requested updates will be visible to everyone who can read that thread"
              : "Connect once, then use your chats and bots from Slack"}
          </DialogDescription>
        </DialogHeader>
        {(status.isLoading || link.isLoading) && (
          <p role="status" className="text-sm text-muted-foreground">
            Checking connection…
          </p>
        )}
        {open && (editing || status.data?.configured === false) && (
          <SlackSetup
            api={api}
            onConnected={() => {
              setEditing(false);
              client.setQueryData(statusKey, {
                configured: true,
                connected: false,
              });
              void client.invalidateQueries({
                queryKey: ["agent-workspace", scope],
              });
            }}
          />
        )}
        {!editing && status.data?.configured && !status.data.connected && (
          <p role="status" className="text-sm text-muted-foreground">
            Credentials saved. Waiting for Slack to connect…{" "}
            <Button variant="link" size="sm" onClick={() => setEditing(true)}>
              Check setup
            </Button>
          </p>
        )}
        {!editing && status.data?.connected && !chatId && (
          <div className="space-y-3 text-sm">
            <p>
              Slack is connected. Open a chat or bot and choose Slack to link
              its conversation to a thread
            </p>
            <Button variant="ghost" size="sm" onClick={() => setEditing(true)}>
              Update connection
            </Button>
          </div>
        )}
        {!editing &&
          status.data?.connected &&
          chatId &&
          (link.data?.linked ? (
            <div className="space-y-4">
              <p className="text-sm">
                This conversation is connected. The person who linked the thread
                can send instructions to the agent there
              </p>
              <Button
                variant="outline"
                disabled={unlink.isPending}
                onClick={() => unlink.mutate()}
              >
                Disconnect thread
              </Button>
            </div>
          ) : (
            link.data && (
              <div className="space-y-4">
                <p className="text-sm text-muted-foreground">
                  Invite the Lens app to your channel. Generate a command, then
                  mention the app with that command in the thread you want to
                  use
                </p>
                {code.data && (
                  <>
                    <pre className="overflow-x-auto rounded-md border bg-muted/40 p-3 text-xs">
                      {code.data.command}
                    </pre>
                    <p className="text-xs text-muted-foreground">
                      One use. Expires at{" "}
                      {new Date(code.data.expires_at).toLocaleTimeString()}
                    </p>
                    <Button
                      variant="outline"
                      disabled={copy.isPending}
                      onClick={() => copy.mutate()}
                    >
                      <Copy className="size-3.5" />
                      Copy command
                    </Button>
                  </>
                )}
                <Button
                  variant={code.data ? "ghost" : "default"}
                  disabled={code.isPending}
                  onClick={() => code.mutate()}
                >
                  {code.isPending && (
                    <Loader2 className="size-3.5 animate-spin" />
                  )}
                  {code.data ? "Generate another command" : "Generate command"}
                </Button>
              </div>
            )
          ))}
        {(status.error ||
          link.error ||
          code.error ||
          unlink.error ||
          copy.error) && (
          <p role="alert" className="text-sm text-destructive">
            {status.error?.message ??
              link.error?.message ??
              code.error?.message ??
              unlink.error?.message ??
              copy.error?.message}
          </p>
        )}
      </DialogContent>
    </Dialog>
  );
}

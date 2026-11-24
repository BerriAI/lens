import { useState } from "react";
import { useMutation, useQuery } from "@tanstack/react-query";
import { Copy, Loader2 } from "lucide-react";
import { Button } from "../../ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "../../ui/dialog";
import { copyToClipboard } from "../../../utils/dataUtils";
import type { WorkspaceApi } from "./Chat";

export function SlackConnection({
  api,
  scope,
  chatId,
}: {
  api: WorkspaceApi;
  scope: string;
  chatId: string;
}) {
  const [open, setOpen] = useState(false);
  const link = useQuery({
    queryKey: ["agent-workspace", scope, "slack", chatId],
    queryFn: () => api.slackLink(chatId),
    enabled: open,
    refetchInterval: open ? 2000 : false,
  });
  const code = useMutation({ mutationFn: () => api.slackCode(chatId) });
  const unlink = useMutation({
    mutationFn: () => api.unlinkSlack(chatId),
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
        if (!value) code.reset();
      }}
    >
      <Button variant="outline" size="sm" onClick={() => setOpen(true)}>
        Slack
      </Button>
      <DialogContent className="sm:max-w-lg">
        <DialogHeader>
          <DialogTitle>Continue in Slack</DialogTitle>
          <DialogDescription>
            Link a Slack thread to this conversation. Replies and requested
            updates will be visible to everyone who can read that thread
          </DialogDescription>
        </DialogHeader>
        {link.isLoading && (
          <p role="status" className="text-sm text-muted-foreground">
            Checking connection…
          </p>
        )}
        {link.data?.linked ? (
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
                mention the app with that command in the thread you want to use
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
        )}
        {(link.error || code.error || unlink.error || copy.error) && (
          <p role="alert" className="text-sm text-destructive">
            {link.error?.message ??
              code.error?.message ??
              unlink.error?.message ??
              copy.error?.message}
          </p>
        )}
      </DialogContent>
    </Dialog>
  );
}

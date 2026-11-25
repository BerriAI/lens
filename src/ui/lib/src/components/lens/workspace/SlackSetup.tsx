import { useState } from "react";
import { useMutation } from "@tanstack/react-query";
import { ExternalLink, Loader2 } from "lucide-react";
import { Button, buttonVariants } from "../../ui/button";
import { Input } from "../../ui/input";
import type { WorkspaceApi } from "./Chat";

const manifest = {
  display_information: {
    name: "Lens",
    description: "Talk to your Lens agents and receive trace findings",
  },
  features: {
    bot_user: { display_name: "Lens", always_online: true },
    app_home: {
      messages_tab_enabled: true,
      messages_tab_read_only_enabled: false,
    },
  },
  oauth_config: {
    scopes: {
      bot: [
        "app_mentions:read",
        "chat:write",
        "im:history",
        "channels:history",
        "groups:history",
      ],
    },
  },
  settings: {
    socket_mode_enabled: true,
    event_subscriptions: {
      bot_events: [
        "app_mention",
        "message.im",
        "message.channels",
        "message.groups",
      ],
    },
  },
};

export function SlackSetup({
  api,
  onConnected,
}: {
  api: WorkspaceApi;
  onConnected: () => void;
}) {
  const [botToken, setBotToken] = useState("");
  const [appToken, setAppToken] = useState("");
  const connect = useMutation({
    gcTime: 0,
    mutationFn: () => api.connectSlack(botToken.trim(), appToken.trim()),
    onSuccess: () => {
      setBotToken("");
      setAppToken("");
      onConnected();
    },
  });
  return (
    <form
      className="space-y-5"
      onSubmit={(event) => {
        event.preventDefault();
        connect.mutate();
      }}
    >
      <div className="space-y-2 text-sm">
        <p>
          Create your Slack app with Lens’s permissions and events already
          filled in. Choose your workspace, then select{" "}
          <strong>Install to Workspace</strong> in Slack
        </p>
        <a
          className={buttonVariants({ variant: "outline", size: "sm" })}
          href={`https://api.slack.com/apps?new_app=1&manifest_json=${encodeURIComponent(JSON.stringify(manifest))}`}
          target="_blank"
          rel="noreferrer"
        >
          Create Slack app <ExternalLink className="size-3.5" />
        </a>
      </div>
      <div className="space-y-2">
        <label htmlFor="slack-bot-token" className="text-sm font-medium">
          Bot token
        </label>
        <Input
          id="slack-bot-token"
          type="password"
          autoComplete="off"
          spellCheck={false}
          value={botToken}
          onChange={(event) => setBotToken(event.target.value)}
          placeholder="xoxb-…"
          required
        />
        <p className="text-xs text-muted-foreground">
          Copy the Bot User OAuth Token from OAuth &amp; Permissions in your
          Slack app
        </p>
      </div>
      <div className="space-y-2">
        <label htmlFor="slack-app-token" className="text-sm font-medium">
          App-level token
        </label>
        <Input
          id="slack-app-token"
          type="password"
          autoComplete="off"
          spellCheck={false}
          value={appToken}
          onChange={(event) => setAppToken(event.target.value)}
          placeholder="xapp-…"
          required
        />
        <p className="text-xs text-muted-foreground">
          Under Basic Information, then App-Level Tokens, generate a token with
          the connections:write scope
        </p>
      </div>
      <p className="text-xs text-muted-foreground">
        One-time setup for this Lens workspace. Credentials are encrypted on the
        server. You choose which chats to link; their replies are visible to
        everyone in the Slack thread
      </p>
      {connect.error && (
        <p role="alert" className="text-sm text-destructive">
          {connect.error.message}
        </p>
      )}
      <Button
        type="submit"
        size="sm"
        disabled={connect.isPending || !botToken.trim() || !appToken.trim()}
      >
        {connect.isPending && <Loader2 className="size-3.5 animate-spin" />}
        {connect.isPending ? "Checking Slack…" : "Connect Slack"}
      </Button>
    </form>
  );
}

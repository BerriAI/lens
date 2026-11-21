"use client";

import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useId, useState } from "react";
import { Button } from "../../../ui/button";
import { Input } from "../../../ui/input";
import { Checkbox } from "../../../ui/checkbox";
import { useLensApi } from "../../data/LensServices";
import { DEFAULT_SIGNAL_THRESHOLD } from "../../model/signals";
import type { SignalAlertView, SignalConfig } from "../../model/types";
import { useOptionalOnboarding } from "../../onboarding/OnboardingContext";
import { StatusDot } from "../../../shared/StatusDot";

export function SignalAlertSettings({ signals }: { signals: SignalConfig }) {
  const api = useLensApi();
  const onboarding = useOptionalOnboarding();
  const readOnly = !onboarding?.canInvestigate || onboarding.readOnly;
  const query = useQuery({
    queryKey: ["signal-alerts", api.scope],
    queryFn: () => api.signalAlerts(),
    refetchOnWindowFocus: false,
    enabled: !readOnly,
  });
  const rules = useQuery({
    queryKey: ["alert-rules", api.scope],
    queryFn: () => api.alertRules(),
    enabled: !readOnly,
  });
  const rulesEnabled = Boolean(rules.data?.rules.some((rule) => rule.enabled));
  return (
    <div className="mx-auto w-full max-w-4xl space-y-6">
      <div className="flex flex-wrap items-start justify-between gap-3">
        <div className="space-y-1">
          <h3 className="text-sm font-medium">Send alerts to Slack</h3>
          <p className="text-sm text-muted-foreground">
            {readOnly
              ? "An administrator can set up alerts"
              : query.data?.enabled
                ? "Sending alerts to Slack"
                : query.data?.connected
                  ? "Alerts paused. Your Slack connection is saved"
                  : "Get a Slack message when a rule or model signal matches a new run"}
          </p>
        </div>
        {query.data && !readOnly && (
          <p className="inline-flex items-center gap-2 text-xs text-muted-foreground">
            <StatusDot state={query.data.enabled ? "ok" : "off"} />
            {query.data.enabled ? "Enabled" : query.data.connected ? "Paused" : "Not connected"}
          </p>
        )}
      </div>
      {!readOnly && (
        <div>
          {query.error || rules.error ? (
            <div role="alert" className="space-y-2 text-sm">
              <p>Could not load alert settings: {query.error?.message ?? rules.error?.message}</p>
              <Button
                variant="outline"
                size="sm"
                onClick={() => {
                  void query.refetch();
                  void rules.refetch();
                }}
              >
                Retry
              </Button>
            </div>
          ) : query.data ? (
            <AlertForm
              key={query.data.revision}
              saved={query.data}
              signals={signals}
              rulesEnabled={rulesEnabled}
            />
          ) : (
            <p role="status" className="text-sm">
              Loading alert settings…
            </p>
          )}
        </div>
      )}
    </div>
  );
}

function deliveryText(status: string) {
  switch (status) {
    case "delivered":
      return "Last alert delivered to Slack";
    case "retrying":
      return "Slack is temporarily unavailable. Lens will retry";
    case "unconfirmed":
      return "Last delivery is unconfirmed. Check Slack before sending another message";
    default:
      return "Last alert failed. Check the webhook and send a test";
  }
}

function AlertForm({
  saved,
  signals,
  rulesEnabled,
}: {
  saved: SignalAlertView;
  signals: SignalConfig;
  rulesEnabled: boolean;
}) {
  const api = useLensApi();
  const queryClient = useQueryClient();
  const webhookId = useId();
  const [webhook, setWebhook] = useState("");
  const [enabled, setEnabled] = useState(saved.connected ? saved.enabled : true);
  const [selected, setSelected] = useState(
    saved.connected ? saved.signal_ids : (signals.signals ?? []).map((signal) => signal.id),
  );
  const selectedSignals = selected.filter((id) => (signals.signals ?? []).some((signal) => signal.id === id));
  const test = useMutation({
    mutationFn: () => api.testSignalAlerts({ webhook_url: webhook.trim() || null }),
  });
  const save = useMutation({
    mutationFn: () =>
      api.saveSignalAlerts({
        revision: saved.revision,
        enabled,
        signal_ids: selectedSignals,
        webhook_url: webhook.trim() || null,
      }),
    onSuccess: (result) => {
      setWebhook("");
      queryClient.setQueryData(["signal-alerts", api.scope], result);
    },
  });
  const ready = rulesEnabled || (Boolean(signals.model) && selectedSignals.length > 0);
  const connected = Boolean(webhook.trim()) || saved.connected;
  const busy = test.isPending || save.isPending;
  return (
    <div className="space-y-6">
      <div className="space-y-1.5">
        <label htmlFor={webhookId} className="text-xs font-medium">
          Slack webhook URL
        </label>
        <Input
          id={webhookId}
          type="password"
          autoComplete="off"
          spellCheck={false}
          value={webhook}
          placeholder={
            saved.connected ? "Webhook saved. Paste a new URL to replace it" : "Paste your Slack incoming webhook URL"
          }
          onChange={(event) => {
            setWebhook(event.target.value);
            test.reset();
          }}
        />
        <p className="text-xs leading-5 text-muted-foreground">
          In your Slack app, open Incoming Webhooks, add a webhook to your workspace, and choose a channel. Paste its
          URL here.{" "}
          <a
            className="underline underline-offset-2"
            href="https://docs.slack.dev/messaging/sending-messages-using-incoming-webhooks/"
            target="_blank"
            rel="noreferrer"
          >
            Slack setup guide
          </a>
        </p>
      </div>
      {rulesEnabled && <p className="text-sm">All enabled rules are included.</p>}
      {Boolean(signals.model) && (
        <fieldset className="space-y-2 border-t pt-5">
          <legend className="sr-only">Alert on these signals</legend>
          <p className="text-sm font-medium">Model signals to include</p>
          <div className="grid gap-x-6 sm:grid-cols-2">
            {(signals.signals ?? []).map((signal) => (
              <label
                key={signal.id}
                htmlFor={`${webhookId}-${signal.id}`}
                className="flex min-w-0 items-start gap-3 border-b py-3 text-sm"
              >
                <Checkbox
                  id={`${webhookId}-${signal.id}`}
                  className="mt-0.5"
                  checked={selected.includes(signal.id)}
                  onCheckedChange={(checked) =>
                    setSelected((current) =>
                      checked ? [...current, signal.id] : current.filter((id) => id !== signal.id),
                    )
                  }
                />
                <span className="min-w-0 wrap-anywhere">{signal.name}</span>
              </label>
            ))}
          </div>
          <p className="text-xs text-muted-foreground">
            Uses your saved match threshold of{" "}
            {Math.round((signals.threshold ?? DEFAULT_SIGNAL_THRESHOLD) * 100)}
            %. Manage the model, questions and threshold in Configure alerts.
          </p>
        </fieldset>
      )}
      <label htmlFor={`${webhookId}-enabled`} className="flex items-center gap-3 text-sm font-medium">
        <Checkbox id={`${webhookId}-enabled`} checked={enabled} onCheckedChange={setEnabled} />
        Enable Slack notifications
      </label>
      <p className="text-xs leading-5 text-muted-foreground">
        Messages include the alert, its result, and a link to the trace. Captured prompts stay in Lens. Only
        runs started after enabling notifications are included, once per alert and run.
      </p>
      {saved.last_delivery && (
        <p role="status" className="text-xs text-muted-foreground">
          {deliveryText(saved.last_delivery.status)} · {new Date(saved.last_delivery.at).toLocaleString()}
        </p>
      )}
      {!ready && enabled && (
        <p className="text-xs text-muted-foreground">
          Enable a rule or configure model signals in Configure alerts first.
        </p>
      )}
      {(test.error || save.error) && (
        <p role="alert" className="text-sm text-destructive">
          {test.error?.message ?? save.error?.message}
        </p>
      )}
      {test.isSuccess && test.data.delivered && (
        <p role="status" className="text-sm">
          Test delivered to Slack. Save settings to apply your choices.
        </p>
      )}
      <div className="flex flex-wrap justify-end gap-2 border-t pt-4">
        <Button variant="outline" disabled={!connected || busy} onClick={() => test.mutate()}>
          {test.isPending ? "Sending…" : "Send test alert"}
        </Button>
        <Button disabled={busy || (enabled && (!ready || !connected))} onClick={() => save.mutate()}>
          {save.isPending ? "Saving…" : "Save alert settings"}
        </Button>
      </div>
    </div>
  );
}

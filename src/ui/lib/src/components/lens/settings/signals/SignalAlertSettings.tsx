"use client";

import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useId, useState } from "react";
import { Button } from "../../../ui/button";
import { Input } from "../../../ui/input";
import { Checkbox } from "../../../ui/checkbox";
import { useLensApi } from "../../data/LensServices";
import type { SignalAlertView, SignalConfig } from "../../model/types";
import { useOptionalOnboarding } from "../../onboarding/OnboardingContext";
import { SettingsCard } from "../SettingsSection";

export function SignalAlertSettings({ signals }: { signals: SignalConfig }) {
  const api = useLensApi();
  const [expanded, setExpanded] = useState(false);
  const readOnly = useOptionalOnboarding()?.readOnly ?? false;
  const query = useQuery({
    queryKey: ["signal-alerts", api.scope],
    queryFn: () => api.signalAlerts(),
    refetchOnWindowFocus: false,
    enabled: !readOnly,
  });
  return (
    <SettingsCard>
      <div className="flex flex-wrap items-start justify-between gap-3">
        <div className="space-y-1">
          <h3 className="text-sm font-medium">Alerts</h3>
          <p className="text-xs text-muted-foreground">
            {readOnly
              ? "An administrator can set up signal alerts"
              : query.data?.enabled
                ? "Sending selected signal matches to Slack"
                : query.data?.connected
                  ? "Alerts paused. Your Slack connection is saved"
                  : "Get a Slack message when a signal matches a new trace"}
          </p>
        </div>
        {!expanded && !readOnly && (
          <Button variant="outline" size="sm" onClick={() => setExpanded(true)}>
            {query.data?.connected ? "Manage alerts" : "Set up alerting"}
          </Button>
        )}
      </div>
      {expanded && (
        <div className="mt-4 border-t pt-4">
          {query.error ? (
            <div role="alert" className="space-y-2 text-sm">
              <p>Could not load alert settings: {query.error.message}</p>
              <Button variant="outline" size="sm" onClick={() => void query.refetch()}>
                Retry
              </Button>
            </div>
          ) : query.data ? (
            <AlertForm key={query.data.revision} saved={query.data} signals={signals} />
          ) : (
            <p role="status" className="text-sm">
              Loading alert settings…
            </p>
          )}
        </div>
      )}
    </SettingsCard>
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

function AlertForm({ saved, signals }: { saved: SignalAlertView; signals: SignalConfig }) {
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
  const ready = Boolean(signals.model) && selectedSignals.length > 0;
  const connected = Boolean(webhook.trim()) || saved.connected;
  const busy = test.isPending || save.isPending;
  return (
    <div className="space-y-4">
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
      <fieldset className="space-y-2">
        <legend className="mb-2 text-xs font-medium">Alert on these signals</legend>
        {(signals.signals ?? []).map((signal) => (
          <label key={signal.id} htmlFor={`${webhookId}-${signal.id}`} className="flex items-center gap-2 text-sm">
            <Checkbox
              id={`${webhookId}-${signal.id}`}
              checked={selected.includes(signal.id)}
              onCheckedChange={(checked) =>
                setSelected((current) =>
                  checked ? [...current, signal.id] : current.filter((id) => id !== signal.id),
                )
              }
            />
            {signal.name}
          </label>
        ))}
        <p className="text-xs text-muted-foreground">
          Uses your saved signal threshold ({Math.round((signals.threshold ?? 0.5) * 100)}%). Save signal changes above
          before configuring alerts.
        </p>
      </fieldset>
      <label htmlFor={`${webhookId}-enabled`} className="flex items-center gap-2 text-sm">
        <Checkbox id={`${webhookId}-enabled`} checked={enabled} onCheckedChange={setEnabled} />
        Enable alerts
      </label>
      <p className="text-xs leading-5 text-muted-foreground">
        Alerts include the signal name, score, and a link to the trace. Captured prompts stay in Lens. Only traces
        started after enabling alerts are included, once per signal and trace, across all agents and teams.
      </p>
      {saved.last_delivery && (
        <p role="status" className="text-xs text-muted-foreground">
          {deliveryText(saved.last_delivery.status)} · {new Date(saved.last_delivery.at).toLocaleString()}
        </p>
      )}
      {!ready && enabled && (
        <p className="text-xs text-muted-foreground">
          Save a signal model above and select at least one signal to enable alerts.
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
      <div className="flex flex-wrap justify-end gap-2">
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

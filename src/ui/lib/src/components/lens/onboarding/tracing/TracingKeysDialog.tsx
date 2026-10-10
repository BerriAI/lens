"use client";

import { useState } from "react";
import { useQueryClient } from "@tanstack/react-query";
import { Button } from "../../../ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "../../../ui/dialog";
import { apiClient } from "../../../../lib/http/requests";
import { useLensAccessToken } from "../../data/LensServices";
import { useOnboarding } from "../OnboardingContext";
import {
  tracingKeysQuery,
  useTracingKeys,
  type TracingKeyRecord,
} from "./tracingKeys";

export function TracingKeysDialog({
  open,
  onOpenChange,
  onSelect,
  onRevoked,
}: {
  readonly open: boolean;
  readonly onOpenChange: (open: boolean) => void;
  readonly onSelect?: (key: TracingKeyRecord) => void;
  readonly onRevoked?: (id: string) => void;
}) {
  const accessToken = useLensAccessToken();
  const { canMintTracingKey, readOnly } = useOnboarding();
  const keys = useTracingKeys(accessToken, open);
  const cache = useQueryClient();
  const [confirm, setConfirm] = useState<string | null>(null);
  const [revoking, setRevoking] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const revoke = async (id: string) => {
    setRevoking(true);
    setError(null);
    try {
      await apiClient.delete(`/lens/tracing/keys/${encodeURIComponent(id)}`, {
        accessToken,
      });
      cache.setQueryData<TracingKeyRecord[]>(
        tracingKeysQuery(accessToken),
        (current) => current?.filter((key) => key.id !== id),
      );
      onRevoked?.(id);
      setConfirm(null);
      void cache.invalidateQueries({ queryKey: tracingKeysQuery(accessToken) });
    } catch {
      setError("Could not revoke the tracing key. Try again.");
    } finally {
      setRevoking(false);
    }
  };
  return (
    <Dialog
      open={open}
      onOpenChange={(next) => {
        onOpenChange(next);
        if (!next) {
          setConfirm(null);
          setError(null);
        }
      }}
    >
      <DialogContent className="sm:max-w-xl">
        <DialogHeader>
          <DialogTitle>Manage tracing keys</DialogTitle>
          <DialogDescription>
            Keys send telemetry and check delivery. Existing secret values
            cannot be shown again.
          </DialogDescription>
        </DialogHeader>
        {keys.isPending ? (
          <p role="status">Loading tracing keys…</p>
        ) : keys.error ? (
          <div role="alert" className="space-y-3">
            <p>Could not load tracing keys.</p>
            <Button variant="outline" onClick={() => void keys.refetch()}>
              Retry
            </Button>
          </div>
        ) : (
          <div className="max-h-96 space-y-3 overflow-y-auto">
            {keys.data?.length === 0 && (
              <p className="text-muted-foreground">
                No tracing keys yet. Create one on Home to connect an agent.
              </p>
            )}
            {keys.data?.map((key) => {
              const expired =
                key.expires_at !== null && key.expires_at * 1000 <= Date.now();
              return (
                <section
                  key={key.id}
                  aria-label={key.name}
                  className="rounded-lg border p-4"
                >
                  <p className="font-medium break-words">{key.name}</p>
                  <p className="mt-1 text-xs text-muted-foreground">
                    Created {new Date(key.created_at).toLocaleString()}
                    {expired
                      ? " · Expired"
                      : key.expires_at
                        ? ` · Expires ${new Date(key.expires_at * 1000).toLocaleDateString()}`
                        : " · No expiry"}
                  </p>
                  {confirm === key.id ? (
                    <div className="mt-3 space-y-3">
                      <p>
                        Agents using this key will stop sending traces.
                        Revocation cannot be undone.
                      </p>
                      <div className="flex flex-wrap gap-2">
                        <Button
                          variant="destructive"
                          disabled={revoking}
                          onClick={() => void revoke(key.id)}
                        >
                          {revoking ? "Revoking…" : "Confirm revoke"}
                        </Button>
                        <Button
                          variant="outline"
                          disabled={revoking}
                          onClick={() => setConfirm(null)}
                        >
                          Cancel
                        </Button>
                      </div>
                    </div>
                  ) : (
                    <div className="mt-3 flex flex-wrap gap-2">
                      {onSelect && !expired && (
                        <Button
                          size="sm"
                          variant="outline"
                          onClick={() => {
                            onSelect(key);
                            onOpenChange(false);
                          }}
                        >
                          Use key in my project
                        </Button>
                      )}
                      {canMintTracingKey && !readOnly && (
                        <Button
                          size="sm"
                          variant="ghost"
                          onClick={() => setConfirm(key.id)}
                        >
                          Revoke
                        </Button>
                      )}
                    </div>
                  )}
                </section>
              );
            })}
          </div>
        )}
        {error && (
          <p role="alert" className="text-destructive">
            {error}
          </p>
        )}
      </DialogContent>
    </Dialog>
  );
}

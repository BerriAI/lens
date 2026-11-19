"use client";

import { useState } from "react";
import { useQueryClient } from "@tanstack/react-query";
import { Button } from "../../../ui/button";
import { Input } from "../../../ui/input";
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
  createTracingKey,
  tracingKeysQuery,
  useTracingKeys,
  type TracingKeyRecord,
} from "./tracingKeys";
import { CodeBlock } from "./TracingSetupCard";

function CreateUserKey() {
  const accessToken = useLensAccessToken();
  const cache = useQueryClient();
  const [name, setName] = useState("");
  const [userId, setUserId] = useState("");
  const [creating, setCreating] = useState(false);
  const [secret, setSecret] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const create = async () => {
    if (!name.trim() || !userId.trim() || creating) return;
    setCreating(true);
    setError(null);
    setSecret(null);
    try {
      const created = await createTracingKey(accessToken, name.trim(), userId);
      cache.setQueryData<TracingKeyRecord[]>(
        tracingKeysQuery(accessToken),
        (current) => [...(current ?? []), created.record],
      );
      setSecret(created.key);
      setName("");
      setUserId("");
    } catch {
      setError("Could not create the tracing key. Try again.");
    } finally {
      setCreating(false);
    }
  };
  return (
    <section
      aria-label="Create key for a user"
      className="space-y-3 rounded-lg border p-4"
    >
      <h3 className="text-sm font-medium">Create key for a user</h3>
      <form
        className="space-y-3"
        onSubmit={(event) => {
          event.preventDefault();
          void create();
        }}
      >
        <label className="block space-y-1 text-sm">
          <span>Key name</span>
          <Input
            required
            maxLength={128}
            value={name}
            onChange={(event) => setName(event.target.value)}
            disabled={creating}
          />
        </label>
        <label className="block space-y-1 text-sm">
          <span>User ID</span>
          <Input
            required
            maxLength={256}
            value={userId}
            onChange={(event) => setUserId(event.target.value)}
            disabled={creating}
          />
        </label>
        <Button
          type="submit"
          size="sm"
          disabled={creating || !name.trim() || !userId.trim()}
        >
          {creating ? "Creating…" : "Create key"}
        </Button>
      </form>
      {secret && (
        <div className="space-y-2" role="status">
          <p className="text-sm">
            Save this key now. It cannot be shown again after you close this
            dialog.
          </p>
          <CodeBlock code={secret} copyLabel="Copy tracing key" />
          <p className="text-xs text-muted-foreground">
            Store this key in LITELLM_TRACING_KEY in the agent’s environment.
          </p>
        </div>
      )}
      {error && (
        <p role="alert" className="text-destructive">
          {error}
        </p>
      )}
    </section>
  );
}

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
  const canCreate = canMintTracingKey && !readOnly;
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
        {open && canCreate && <CreateUserKey />}
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
                  <p className="text-xs text-muted-foreground break-words">
                    Assigned to {key.tenant.user_id}
                  </p>
                  {key.created_by && key.created_by !== key.tenant.user_id && (
                    <p className="text-xs text-muted-foreground break-words">
                      Created by {key.created_by}
                    </p>
                  )}
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

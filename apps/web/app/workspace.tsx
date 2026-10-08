"use client";

import { useState, type FormEvent } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { parseAsBoolean, useQueryState } from "nuqs";
import { z } from "zod";
import { configureLensHttp, LensWorkspace } from "@litellm/lens-ui";
import { ApiError, createApiClient } from "@litellm/lens-ui/http";

const baseUrl = () => process.env.NEXT_PUBLIC_LENS_API_URL ?? "";
const api = createApiClient({
  getBaseUrl: baseUrl,
  getAuthHeaderName: () => "Authorization",
});
const sessionSchema = z.object({ user_id: z.string(), user_role: z.string() });
const sessionKey = ["lens-session"];

configureLensHttp({ getBaseUrl: baseUrl, getAuthToken: () => null });

export function Workspace() {
  const [demo] = useQueryState("demo", parseAsBoolean.withDefault(false));
  const session = useQuery({
    queryKey: sessionKey,
    queryFn: async () => sessionSchema.parse(await api.get("/auth/session")),
    enabled: !demo,
    retry: false,
  });
  if (demo) return <Surface userRole="proxy_admin_viewer" readOnly />;
  if (session.isPending)
    return (
      <p role="status" className="p-8 text-sm text-muted-foreground">
        Opening Lens…
      </p>
    );
  if (session.data)
    return (
      <Surface
        userRole={session.data.user_role}
        readOnly={session.data.user_role === "proxy_admin_viewer"}
      />
    );
  if (session.error instanceof ApiError && session.error.status === 401)
    return <SignIn />;
  return (
    <main className="mx-auto flex max-w-md flex-col gap-4 p-8">
      <h1 className="text-lg font-semibold">Cannot reach Lens</h1>
      <p className="text-sm text-muted-foreground">
        Check that the Lens API is running, then retry.
      </p>
      <button
        className="rounded-md border px-3 py-2 text-sm"
        onClick={() => void session.refetch()}
      >
        Retry
      </button>
      <a href="?demo=true" className="text-sm underline">
        Explore demo data
      </a>
    </main>
  );
}

function Surface({
  userRole,
  readOnly,
}: {
  userRole: string;
  readOnly: boolean;
}) {
  return (
    <div className="flex h-dvh min-h-0 flex-col">
      <LensWorkspace accessToken="" userRole={userRole} readOnly={readOnly} />
    </div>
  );
}

function SignIn() {
  const client = useQueryClient();
  const [token, setToken] = useState("");
  const login = useMutation({
    mutationFn: async (value: string) =>
      sessionSchema.parse(
        await api.post("/auth/session", { body: { token: value } }),
      ),
    onSuccess: (session) => {
      setToken("");
      client.removeQueries({
        predicate: (query) => query.queryKey[0] !== sessionKey[0],
      });
      client.setQueryData(sessionKey, session);
    },
  });
  const submit = (event: FormEvent) => {
    event.preventDefault();
    login.mutate(token);
  };
  return (
    <main className="mx-auto flex max-w-md flex-col gap-5 p-8">
      <h1 className="text-xl font-semibold">Sign in to Lens</h1>
      <form onSubmit={submit} className="flex flex-col gap-3">
        <label htmlFor="setup-token" className="text-sm font-medium">
          Setup token
        </label>
        <input
          id="setup-token"
          type="password"
          autoComplete="current-password"
          required
          value={token}
          onChange={(event) => setToken(event.target.value)}
          className="rounded-md border px-3 py-2"
        />
        <button
          disabled={login.isPending}
          className="rounded-md bg-primary px-3 py-2 text-sm text-primary-foreground"
        >
          {login.isPending ? "Signing in…" : "Sign in"}
        </button>
        {login.isError && (
          <p role="alert" className="text-sm text-destructive">
            {login.error.message}
          </p>
        )}
      </form>
      <a href="?demo=true" className="text-sm underline">
        Explore demo data
      </a>
    </main>
  );
}

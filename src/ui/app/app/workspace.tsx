"use client";

import { useEffect, useState, type FormEvent } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { parseAsBoolean, useQueryState } from "nuqs";
import { z } from "zod";
import {
  configureLensHttp,
  hostedApiPath,
  hostedApiRoot,
  LensHostProvider,
  LensWorkspace,
  type LensHost,
  takeSignInToken,
} from "@litellm/lens-ui";
import litellmMonogram from "../../lib/public/assets/logos/litellm_monogram.svg";
import { ApiError, createApiClient } from "@litellm/lens-ui/http";
import { toast } from "sonner";

const gatewayApiRoot = () =>
  typeof window === "undefined"
    ? null
    : hostedApiRoot(window.location.pathname);
const baseUrl = () =>
  gatewayApiRoot() === null ? (process.env.NEXT_PUBLIC_LENS_API_URL ?? "") : "";
const resolvePath = (path: string) => {
  const root = gatewayApiRoot();
  return root === null ? path : hostedApiPath(root, path);
};
const api = createApiClient({
  getBaseUrl: baseUrl,
  resolvePath,
  getAuthHeaderName: () => "Authorization",
});
const sessionSchema = z.object({
  user_id: z.string(),
  user_name: z.string().optional(),
  user_role: z.string(),
});
const sessionKey = ["lens-session"];
const signInMethodsKey = ["lens-sign-in-methods"];
const signInMethodsSchema = z.object({
  litellm_login_url: z.string().nullable(),
});

const gatewayConnectedSchema = z.object({
  litellm_login_url: z.string(),
  gateway_environment: z.array(z.string()),
});

configureLensHttp({
  getBaseUrl: baseUrl,
  getAuthToken: () => null,
  resolvePath,
});

export function Workspace() {
  const [demo] = useQueryState("demo", parseAsBoolean.withDefault(false));
  const [linkRejected, setLinkRejected] = useState(false);
  const session = useQuery({
    queryKey: sessionKey,
    queryFn: async () => {
      const token =
        gatewayApiRoot() === null
          ? takeSignInToken(window.location, window.history)
          : null;
      const signedIn = token
        ? await api
            .post("/auth/session", { body: { token } })
            .then((opened) => {
              setLinkRejected(false);
              return opened;
            })
            .catch(() => {
              setLinkRejected(true);
              return null;
            })
        : null;
      return sessionSchema.parse(
        signedIn ?? (await api.get("/auth/session", { cache: "no-store" })),
      );
    },
    enabled: !demo,
    retry: false,
  });
  const client = useQueryClient();
  useEffect(() => {
    const signIn = () =>
      void client.invalidateQueries({ queryKey: sessionKey });
    window.addEventListener("hashchange", signIn);
    return () => window.removeEventListener("hashchange", signIn);
  }, [client]);
  const signOut = async () => {
    setLinkRejected(false);
    const signedOut = await api
      .delete("/auth/session")
      .then(() => true)
      .catch(
        (error: unknown) => error instanceof ApiError && error.status === 401,
      );
    if (!signedOut) {
      toast.error("Sign out did not complete. You are still signed in.", {
        description: "Reload the page and try again.",
      });
      return;
    }
    await client.resetQueries({ queryKey: sessionKey });
  };
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
        user={
          gatewayApiRoot() === null
            ? {
                name: session.data.user_name ?? session.data.user_id,
                role: session.data.user_role,
                signOut: () => void signOut(),
              }
            : undefined
        }
      />
    );
  if (gatewayApiRoot() !== null)
    return (
      <main className="lens-workspace mx-auto my-16 w-[calc(100%-2rem)] max-w-md overflow-hidden rounded-xl border bg-card">
        <header className="lens-page-header">
          <h1 className="lens-page-title">Lens is not available</h1>
        </header>
        <div className="lens-page-body flex flex-col gap-4">
          <p className="text-sm text-muted-foreground">
            {session.error instanceof ApiError &&
            [401, 403].includes(session.error.status)
              ? "Sign in to LiteLLM again, then reload this page."
              : "LiteLLM could not reach Lens. Check that Lens is running, then retry."}
          </p>
          <button
            className="rounded-md border px-3 py-2 text-sm"
            onClick={() => void session.refetch()}
          >
            Retry
          </button>
        </div>
      </main>
    );
  if (session.error instanceof ApiError && session.error.status === 401)
    return <SignIn linkRejected={linkRejected} />;
  return (
    <main className="lens-workspace mx-auto my-16 w-[calc(100%-2rem)] max-w-md overflow-hidden rounded-xl border bg-card">
      <header className="lens-page-header">
        <h1 className="lens-page-title">Cannot reach Lens</h1>
      </header>
      <div className="lens-page-body flex flex-col gap-4">
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
      </div>
    </main>
  );
}

function Surface({
  userRole,
  readOnly,
  user,
}: {
  userRole: string;
  readOnly: boolean;
  user?: LensHost["user"];
}) {
  return (
    <div className="flex h-dvh min-h-0 flex-col">
      <LensHostProvider
        host={{ analysis: "deployment", surface: "standalone", user }}
      >
        <LensWorkspace accessToken="" userRole={userRole} readOnly={readOnly} />
      </LensHostProvider>
    </div>
  );
}

const primaryButton =
  "inline-flex items-center justify-center gap-2 rounded-md bg-primary px-3 py-2 text-center text-sm text-primary-foreground";

function LiteLlmLogo() {
  const mask = `url(${litellmMonogram.src}) center / contain no-repeat`;
  return (
    <span
      aria-hidden
      className="size-5 shrink-0 bg-current"
      style={{ mask, WebkitMask: mask }}
    />
  );
}

function ConnectGateway() {
  const client = useQueryClient();
  const [token, setToken] = useState("");
  const [gatewayUrl, setGatewayUrl] = useState("");
  const [apiKey, setApiKey] = useState("");
  const connect = useMutation({
    mutationFn: async () =>
      gatewayConnectedSchema.parse(
        await api.post("/auth/gateway", {
          body: {
            token,
            gateway_url: gatewayUrl,
            api_key: apiKey.trim() || null,
          },
        }),
      ),
    onSuccess: () => {
      setToken("");
      setApiKey("");
      void client.invalidateQueries({ queryKey: signInMethodsKey });
    },
  });
  if (connect.isSuccess) {
    const lines = connect.data.gateway_environment.join("\n");
    return (
      <section
        aria-label="Gateway connected"
        className="flex flex-col gap-3 rounded-md border p-3"
      >
        <p className="text-sm font-medium">Lens is connected</p>
        <p className="text-xs text-muted-foreground">
          Add these lines to your LiteLLM gateway&apos;s environment and restart
          it. Then log in with LiteLLM. The secret is shown only once.
        </p>
        <pre className="overflow-x-auto rounded-md bg-muted p-2 text-xs">
          {lines}
        </pre>
        <button
          type="button"
          className="rounded-md border px-3 py-2 text-sm"
          onClick={() =>
            void navigator.clipboard
              .writeText(lines)
              .then(() => toast.success("Copied"))
          }
        >
          Copy
        </button>
      </section>
    );
  }
  return (
    <form
      aria-label="Set up your gateway"
      onSubmit={(event) => {
        event.preventDefault();
        connect.mutate();
      }}
      className="flex flex-col gap-3 rounded-md border p-3"
    >
      <label htmlFor="connect-gateway-url" className="text-sm font-medium">
        Gateway address
      </label>
      <input
        id="connect-gateway-url"
        type="url"
        required
        placeholder="https://litellm.example.com"
        value={gatewayUrl}
        onChange={(event) => setGatewayUrl(event.target.value)}
        className="rounded-md border px-3 py-2"
      />
      <label htmlFor="connect-api-key" className="text-sm font-medium">
        Gateway API key{" "}
        <span className="font-normal text-muted-foreground">(optional)</span>
      </label>
      <input
        id="connect-api-key"
        type="password"
        autoComplete="off"
        value={apiKey}
        onChange={(event) => setApiKey(event.target.value)}
        className="rounded-md border px-3 py-2"
      />
      <p className="text-xs text-muted-foreground">
        With a key, Lens also uses the gateway&apos;s models for signals and
        findings.
      </p>
      <label htmlFor="connect-token" className="text-sm font-medium">
        Admin token
      </label>
      <input
        id="connect-token"
        type="password"
        autoComplete="current-password"
        required
        value={token}
        onChange={(event) => setToken(event.target.value)}
        className="rounded-md border px-3 py-2"
      />
      <p className="text-xs text-muted-foreground">
        The admin token proves you run this Lens.
      </p>
      <button
        disabled={connect.isPending}
        className="rounded-md border px-3 py-2 text-sm"
      >
        {connect.isPending ? "Connecting…" : "Connect gateway"}
      </button>
      {connect.isError && (
        <p role="alert" className="text-sm text-destructive">
          {connect.error.message}
        </p>
      )}
    </form>
  );
}

function SignIn({ linkRejected }: { linkRejected: boolean }) {
  const client = useQueryClient();
  const [token, setToken] = useState("");
  const [settingUp, setSettingUp] = useState(false);
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
  const methods = useQuery({
    queryKey: signInMethodsKey,
    queryFn: async () =>
      signInMethodsSchema.parse(await api.get("/auth/methods")),
    retry: false,
  });
  const litellmLoginUrl = methods.data?.litellm_login_url;
  const submit = (event: FormEvent) => {
    event.preventDefault();
    login.mutate(token);
  };
  const form = (
    <form onSubmit={submit} className="flex flex-col gap-3">
      <label htmlFor="setup-token" className="text-sm font-medium">
        Admin token
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
        className="rounded-md border px-3 py-2 text-sm"
      >
        {login.isPending ? "Signing in…" : "Sign in"}
      </button>
      {login.isError && (
        <p role="alert" className="text-sm text-destructive">
          {login.error.message}
        </p>
      )}
    </form>
  );
  return (
    <main className="lens-workspace mx-auto my-16 w-[calc(100%-2rem)] max-w-md overflow-hidden rounded-xl border bg-card">
      <header className="lens-page-header">
        <h1 className="lens-page-title">Sign in to Lens</h1>
      </header>
      <div className="lens-page-body flex flex-col gap-5">
        {linkRejected && (
          <p role="alert" className="text-sm text-destructive">
            That sign-in link was not accepted. It may have expired. Sign in
            again.
          </p>
        )}
        {methods.isPending ? null : (
          <>
            {litellmLoginUrl ? (
              <a href={litellmLoginUrl} className={primaryButton}>
                <LiteLlmLogo />
                Log in with LiteLLM
              </a>
            ) : (
              <button
                type="button"
                className={primaryButton}
                onClick={() => setSettingUp(true)}
              >
                <LiteLlmLogo />
                Log in with LiteLLM
              </button>
            )}
            <p className="text-center text-xs text-muted-foreground">
              Don&apos;t have your gateway set up?{" "}
              <button
                type="button"
                aria-expanded={settingUp}
                className="text-foreground underline underline-offset-4"
                onClick={() => setSettingUp(!settingUp)}
              >
                Set up
              </button>
            </p>
            {settingUp && <ConnectGateway />}
            <details className="group">
              <summary className="flex cursor-pointer list-none items-center justify-center gap-1 text-xs text-muted-foreground hover:text-foreground [&::-webkit-details-marker]:hidden">
                Legacy admin token
                <svg
                  aria-hidden="true"
                  viewBox="0 0 16 16"
                  className="size-3 transition-transform group-open:rotate-180"
                  fill="none"
                  stroke="currentColor"
                  strokeWidth="1.5"
                >
                  <path
                    d="m4 6 4 4 4-4"
                    strokeLinecap="round"
                    strokeLinejoin="round"
                  />
                </svg>
              </summary>
              <div className="pt-4">{form}</div>
            </details>
          </>
        )}
        <a href="?demo=true" className="text-sm underline">
          Explore demo data
        </a>
      </div>
    </main>
  );
}

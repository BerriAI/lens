"use client";

import { useEffect, useState } from "react";
import { useSearchParams } from "next/navigation";
import { z } from "zod";
import {
  configureLensHttp,
  LensHostProvider,
  LensWorkspace,
  type LensHost,
} from "@litellm/lens-ui";

export const EMBED_PROTOCOL = 1;

const hostSchema = z.object({
  type: z.literal("lens:host"),
  protocol: z.literal(EMBED_PROTOCOL),
  apiBaseUrl: z.string(),
  authToken: z.string().min(1),
  authHeaderName: z.string().min(1),
  serverRootPath: z.string(),
  userRole: z.string(),
  readOnly: z.boolean(),
});
type Host = z.infer<typeof hostSchema>;

const lensHost: LensHost = { surface: "embedded", analysis: "deployment" };

export function EmbeddedWorkspace() {
  const [host, setHost] = useState<Host | null>(null);
  const [framed, setFramed] = useState(true);
  const search = useSearchParams().toString();
  useEffect(() => {
    const receive = (event: MessageEvent) => {
      if (
        event.origin !== window.location.origin ||
        event.source !== window.parent
      )
        return;
      const message = hostSchema.safeParse(event.data);
      if (!message.success) return;
      configureLensHttp({
        getBaseUrl: () => message.data.apiBaseUrl,
        getAuthToken: () => message.data.authToken,
        getAuthHeaderName: () => message.data.authHeaderName,
        serverRootPath: message.data.serverRootPath,
      });
      setHost(message.data);
    };
    setFramed(window.parent !== window);
    window.addEventListener("message", receive);
    window.parent.postMessage(
      { type: "lens:ready", protocol: EMBED_PROTOCOL },
      window.location.origin,
    );
    return () => window.removeEventListener("message", receive);
  }, []);
  useEffect(() => {
    window.parent.postMessage(
      { type: "lens:location", protocol: EMBED_PROTOCOL, search },
      window.location.origin,
    );
  }, [search]);
  if (!framed)
    return (
      <p className="p-8 text-sm text-muted-foreground">
        This page is the Lens workspace for the LiteLLM dashboard. Open Lens
        from the dashboard to use it.
      </p>
    );
  if (!host)
    return (
      <p role="status" className="p-8 text-sm text-muted-foreground">
        Opening Lens…
      </p>
    );
  return (
    <div className="flex h-dvh min-h-0 flex-col">
      <LensHostProvider host={lensHost}>
        <LensWorkspace
          accessToken={host.authToken}
          userRole={host.userRole}
          readOnly={host.readOnly}
        />
      </LensHostProvider>
    </div>
  );
}

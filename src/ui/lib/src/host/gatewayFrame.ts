"use client";

import { useEffect, useState } from "react";
import { z } from "zod";
import { hostedApiRoot } from "../lib/http/resolveApiBase";

export const GATEWAY_FRAME_CHANNEL = "litellm-lens:v1";

type ShareLocation = Pick<Location, "origin" | "pathname" | "search">;

export function gatewayFrameShareLocation(
  location: ShareLocation = window.location,
  parent: { readonly location: ShareLocation } = window.parent,
): ShareLocation {
  if (hostedApiRoot(location.pathname) === null) return location;
  try {
    const target = parent.location;
    return target.origin === location.origin
      ? {
          origin: location.origin,
          pathname: target.pathname,
          search: location.search,
        }
      : location;
  } catch {
    return location;
  }
}

const gatewayFrameCommand = z.discriminatedUnion("type", [
  z.object({ type: z.literal("theme"), theme: z.enum(["light", "dark"]) }),
  z.object({ type: z.literal("tab"), value: z.string() }),
  z.object({ type: z.literal("agent"), value: z.string() }),
  z.object({
    type: z.literal("auth"),
    accessToken: z.string().regex(/^[\x21-\x7e]+$/),
    headerName: z.string().regex(/^[!#$%&'*+.^_`|~0-9A-Za-z-]+$/),
  }),
]);
export type GatewayFrameCommand = z.infer<typeof gatewayFrameCommand>;
export type GatewayFrameAuth = Extract<GatewayFrameCommand, { type: "auth" }>;

const gatewayFrameMessage = z.object({
  channel: z.literal(GATEWAY_FRAME_CHANNEL),
  command: gatewayFrameCommand,
});

export function onGatewayFrameCommand(
  receive: (command: GatewayFrameCommand) => void,
): (() => void) | undefined {
  if (
    window.parent === window ||
    hostedApiRoot(window.location.pathname) === null
  )
    return;
  const listener = (event: MessageEvent<unknown>) => {
    if (
      event.origin !== window.location.origin ||
      event.source !== window.parent
    )
      return;
    const parsed = gatewayFrameMessage.safeParse(event.data);
    if (parsed.success) receive(parsed.data.command);
  };
  window.addEventListener("message", listener);
  return () => window.removeEventListener("message", listener);
}

export function useGatewayFrameTheme() {
  const [theme, setTheme] = useState<"light" | "dark">();
  useEffect(() => {
    const stop = onGatewayFrameCommand((command) => {
      if (command.type === "theme") setTheme(command.theme);
    });
    if (!stop) return;
    window.parent.postMessage(
      { channel: GATEWAY_FRAME_CHANNEL, type: "ready" },
      window.location.origin,
    );
    return stop;
  }, []);
  return theme;
}

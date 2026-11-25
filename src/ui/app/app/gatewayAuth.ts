"use client";

import { useEffect, useState } from "react";
import {
  GATEWAY_FRAME_CHANNEL,
  hostedApiRoot,
  onGatewayFrameCommand,
  type GatewayFrameAuth,
} from "@litellm/lens-ui";

type GatewayAuthState =
  | { status: "waiting" }
  | { status: "unavailable" }
  | { status: "ready"; auth: GatewayFrameAuth | null };

export function useGatewayAuth(
  configure: (auth: GatewayFrameAuth | null) => void,
) {
  const [required] = useState(
    () =>
      typeof window !== "undefined" &&
      hostedApiRoot(window.location.pathname) !== null &&
      window.frameElement?.getAttribute("data-lens-auth") === "bearer-v1",
  );
  const [state, setState] = useState<GatewayAuthState>({ status: "waiting" });
  const [attempt, setAttempt] = useState(0);

  useEffect(() => {
    if (!required) {
      configure(null);
      setState({ status: "ready", auth: null });
      return;
    }
    const stop = onGatewayFrameCommand((command) => {
      if (command.type !== "auth") return;
      stop?.();
      window.clearTimeout(timeout);
      configure(command);
      setState({ status: "ready", auth: command });
    });
    const timeout = window.setTimeout(() => {
      stop?.();
      setState({ status: "unavailable" });
    }, 10_000);
    if (stop) {
      window.parent.postMessage(
        { channel: GATEWAY_FRAME_CHANNEL, type: "auth-ready" },
        window.location.origin,
      );
    }
    return () => {
      stop?.();
      window.clearTimeout(timeout);
    };
  }, [required, attempt, configure]);

  const retry = () => {
    setState({ status: "waiting" });
    setAttempt((current) => current + 1);
  };
  return { ...state, retry };
}

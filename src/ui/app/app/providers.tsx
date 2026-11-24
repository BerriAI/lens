"use client";

import { useState, type ReactNode } from "react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { ApiError } from "@litellm/lens-ui/http";
import { ThemeProvider } from "next-themes";
import { NuqsAdapter } from "nuqs/adapters/next/app";
import { HotkeysProvider } from "react-hotkeys-hook";
import { Toaster, useGatewayFrameTheme } from "@litellm/lens-ui";

export function Providers({ children }: { children: ReactNode }) {
  const gatewayTheme = useGatewayFrameTheme();
  const [client] = useState(
    () =>
      new QueryClient({
        defaultOptions: {
          queries: {
            retry: (count, error) =>
              count < 3 &&
              (!(error instanceof ApiError) || error.status >= 500),
          },
        },
      }),
  );
  return (
    <ThemeProvider
      attribute="class"
      defaultTheme="light"
      forcedTheme={gatewayTheme}
      enableSystem
      disableTransitionOnChange
    >
      <NuqsAdapter>
        <QueryClientProvider client={client}>
          <HotkeysProvider>{children}</HotkeysProvider>
          <Toaster />
        </QueryClientProvider>
      </NuqsAdapter>
    </ThemeProvider>
  );
}

"use client";

import {
  createContext,
  useContext,
  type ComponentType,
  type ReactNode,
} from "react";
import type { LogEntry } from "../components/logs/types";

export interface SpendLogLookup {
  readonly accessToken: string;
  readonly start_date: string;
  readonly end_date: string;
  readonly page: number;
  readonly page_size: number;
  readonly params: { readonly request_id: string };
}

export interface SpendLogDrawerProps {
  readonly open: boolean;
  readonly onClose: () => void;
  readonly logEntry: LogEntry | null;
  readonly accessToken: string;
  readonly backTo: { readonly label: string; readonly icon: ReactNode };
}

export interface LensHost {
  readonly spendLogs?: {
    readonly lookup: (
      query: SpendLogLookup,
    ) => Promise<{ readonly data: readonly LogEntry[] }>;
    readonly Drawer: ComponentType<SpendLogDrawerProps>;
  };
}

const LensHostContext = createContext<LensHost>({});

export function LensHostProvider({
  host,
  children,
}: {
  host: LensHost;
  children: ReactNode;
}) {
  return (
    <LensHostContext.Provider value={host}>{children}</LensHostContext.Provider>
  );
}

export const useLensHost = (): LensHost => useContext(LensHostContext);

"use client";

import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { apiClient } from "../../../lib/http/requests";
import { useLensAccessToken, useLensApi } from "../data/LensServices";

export interface GitHubRepositoryOption {
  readonly id: number;
  readonly full_name: string;
  readonly installation_id: number;
  readonly default_branch: string;
}

export interface GitHubConnection {
  readonly agent: string;
  readonly repository_id: number;
  readonly repository: string;
  readonly installation_id: number;
  readonly default_branch: string;
  readonly connected_at: string;
}

export interface GitHubConnectionStatus extends GitHubConnection {
  readonly available: boolean;
  readonly availability_error?: string | null;
}

interface GitHubStatus {
  readonly configured: boolean;
  readonly app_slug: string | null;
  readonly connection: GitHubConnectionStatus | null;
}

export interface GitHubAuthorization {
  readonly status: "pending" | "ready" | "failed" | "connected";
  readonly repositories: readonly GitHubRepositoryOption[];
  readonly error?: string | null;
  readonly installation_url?: string | null;
}

interface GitHubAuthorizationStart {
  readonly authorization_url: string;
  readonly authorization_id: string;
  readonly expires_at: string;
}

const HEADERS = { "X-Lens-Contract": "1" };

export function useGitHubConnection(agent: string, authorizationId?: string | null) {
  const accessToken = useLensAccessToken();
  const { scope } = useLensApi();
  const client = useQueryClient();
  const options = { accessToken, headers: HEADERS };
  const key = ["lensGitHub", scope, agent] as const;
  const status = useQuery({
    queryKey: key,
    queryFn: () =>
      apiClient.get<GitHubStatus>("/lens/github/status", {
        ...options,
        query: { agent },
      }),
    retry: false,
    refetchOnMount: "always",
  });
  const authorization = useQuery({
    queryKey: [...key, "authorization", authorizationId],
    queryFn: () =>
      apiClient.get<GitHubAuthorization>(
        `/lens/github/authorizations/${encodeURIComponent(authorizationId!)}`,
        options,
      ),
    enabled: Boolean(authorizationId),
    retry: false,
    refetchInterval: (query) => (query.state.data?.status === "pending" ? 2000 : false),
  });
  const start = useMutation({
    mutationFn: (install: boolean) =>
      apiClient.post<GitHubAuthorizationStart>("/lens/github/authorize", {
        ...options,
        body: { agent, install },
      }),
    retry: false,
  });
  const connect = useMutation({
    mutationFn: (repositoryId: number) =>
      apiClient.put<GitHubConnection>(`/lens/github/connections/${encodeURIComponent(agent)}`, {
        ...options,
        body: {
          authorization_id: authorizationId,
          repository_id: repositoryId,
        },
      }),
    onSuccess: () => client.invalidateQueries({ queryKey: key }),
    retry: false,
  });
  const disconnect = useMutation({
    mutationFn: () => apiClient.delete<void>(`/lens/github/connections/${encodeURIComponent(agent)}`, options),
    onSuccess: () => client.invalidateQueries({ queryKey: key }),
    retry: false,
  });
  return { status, authorization, start, connect, disconnect };
}

export function openGitHubAuthorization(url: string): void {
  const destination = new URL(url);
  if (
    destination.protocol !== "https:" ||
    destination.hostname !== "github.com" ||
    destination.username ||
    destination.password
  )
    throw new Error("Lens returned an invalid GitHub authorization address");
  window.location.assign(destination.href);
}

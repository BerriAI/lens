import { fetchClient } from "../../../lib/http/api";
import type {
  ConversationState,
  AnnotationSelection,
} from "../../../lib/http/agentWorkspace";

const required = <T>(data: T | undefined): T => {
  if (data === undefined) throw new Error("Lens returned an empty response");
  return data;
};

export function workspaceApi(accessToken: string) {
  const options = {
    headers: accessToken ? { Authorization: `Bearer ${accessToken}` } : {},
    credentials: "include" as const,
  };
  return {
    slackStatus: async () =>
      required((await fetchClient.GET("/lens/workspace/slack", options)).data),
    connectSlack: async (bot_token: string, app_token: string) =>
      required(
        (
          await fetchClient.POST("/lens/workspace/slack", {
            ...options,
            body: { bot_token, app_token },
          })
        ).data,
      ),
    slackLink: async (id: string) =>
      required(
        (
          await fetchClient.GET("/lens/workspace/conversations/{id}/slack", {
            ...options,
            params: { path: { id } },
          })
        ).data,
      ),
    slackCode: async (id: string) =>
      required(
        (
          await fetchClient.POST("/lens/workspace/conversations/{id}/slack", {
            ...options,
            params: { path: { id } },
          })
        ).data,
      ),
    unlinkSlack: async (id: string) =>
      required(
        (
          await fetchClient.DELETE("/lens/workspace/conversations/{id}/slack", {
            ...options,
            params: { path: { id } },
          })
        ).data,
      ),
    activity: async () =>
      required(
        (await fetchClient.GET("/lens/workspace/activity", options)).data,
      ),
    queryAnnotations: async (filter: AnnotationSelection, after?: number) =>
      required(
        (
          await fetchClient.POST("/lens/workspace/annotations/query", {
            ...options,
            body: { ...filter, after, limit: 100 },
          })
        ).data,
      ),
    models: async () =>
      required((await fetchClient.GET("/lens/workspace/models", options)).data),
    state: async () =>
      required((await fetchClient.GET("/lens/workspace", options)).data),
    conversations: async (actor_id: string) =>
      required(
        (
          await fetchClient.GET("/lens/workspace/conversations", {
            ...options,
            params: { query: { actor_id } },
          })
        ).data,
      ),
    conversation: async (id: string, previous?: ConversationState) => {
      const after = previous?.events.at(-1)?.sequence ?? 0;
      const data = required(
        (
          await fetchClient.GET("/lens/workspace/conversations/{id}", {
            ...options,
            params: { path: { id }, query: { after } },
          })
        ).data,
      );
      return { ...data, events: [...(previous?.events ?? []), ...data.events] };
    },
    createConversation: async (
      title: string,
      actor_id?: string,
      id?: string,
      model?: string,
    ) =>
      required(
        (
          await fetchClient.POST("/lens/workspace/conversations", {
            ...options,
            body: { title, actor_id, id, model },
          })
        ).data,
      ),
    send: async (conversation: string, id: string, text: string) =>
      required(
        (
          await fetchClient.POST(
            "/lens/workspace/conversations/{id}/messages",
            {
              ...options,
              params: { path: { id: conversation } },
              body: { id, text },
            },
          )
        ).data,
      ),
    stop: async (id: string) =>
      required(
        (
          await fetchClient.POST("/lens/workspace/conversations/{id}/stop", {
            ...options,
            params: { path: { id } },
          })
        ).data,
      ),
    createBot: async (
      name: string,
      instructions: string,
      id: string,
      model?: string,
    ) =>
      required(
        (
          await fetchClient.POST("/lens/workspace/bots", {
            ...options,
            body: { name, instructions, id, model },
          })
        ).data,
      ),
    setEnabled: async (id: string, enabled: boolean) =>
      required(
        (
          await fetchClient.PATCH("/lens/workspace/bots/{id}", {
            ...options,
            params: { path: { id } },
            body: { enabled },
          })
        ).data,
      ),
    reports: async (query?: {
      trace_id?: string;
      trace_ref?: string;
      before?: string;
    }) =>
      required(
        (
          await fetchClient.GET("/lens/workspace/reports", {
            ...options,
            params: { query },
          })
        ).data,
      ),
    report: async (id: string) =>
      required(
        (
          await fetchClient.GET("/lens/workspace/reports/{id}", {
            ...options,
            params: { path: { id } },
          })
        ).data,
      ),
    artifact: async (id: string) =>
      required(
        (
          await fetchClient.GET("/lens/workspace/artifacts/{id}", {
            ...options,
            params: { path: { id } },
          })
        ).data,
      ),
    annotations: async (trace_id: string, trace_ref: string, after?: number) =>
      required(
        (
          await fetchClient.POST("/lens/workspace/annotations/query", {
            ...options,
            body: { trace_id, trace_ref, after, limit: 1000 },
          })
        ).data,
      ),
  };
}

import { useRef, useState } from "react";
import { useMutation } from "@tanstack/react-query";
import { Bot, Loader2 } from "lucide-react";
import type { WorkspaceAgent } from "../../../lib/http/agentWorkspace";
import { Button } from "../../ui/button";
import { Input } from "../../ui/input";
import { Textarea } from "../../ui/textarea";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "../../ui/dialog";
import type { WorkspaceApi } from "./Chat";
import { ModelPicker } from "./ModelPicker";

export function Bots({
  bots,
  writable,
  onSelect,
  onCreate,
}: {
  bots: WorkspaceAgent[];
  writable: boolean;
  onSelect: (id: string) => void;
  onCreate: () => void;
}) {
  return (
    <section className="lens-page-body mx-auto w-full max-w-4xl overflow-y-auto p-5 md:p-8">
      <div className="mb-7 flex items-start justify-between gap-4">
        <div>
          <h2 className="lens-page-title">Give a bot an ongoing task</h2>
          <p className="mt-2 max-w-xl text-sm text-muted-foreground">
            Bots keep their context and work while you’re away. Talk to one to
            refine its task, review its work, or change direction
          </p>
        </div>
      </div>
      {bots.length === 0 ? (
        <div className="rounded-lg border px-5 py-8">
          {writable ? (
            <Button size="sm" onClick={onCreate}>
              Create a new bot
            </Button>
          ) : (
            <p className="text-sm">No bots yet</p>
          )}
          <p className="mt-2 max-w-lg text-sm text-muted-foreground">
            Try a bot that reviews new requests, annotates unsuccessful tasks,
            or investigates something worth bringing to your attention
          </p>
        </div>
      ) : (
        <div className="divide-y rounded-lg border">
          {bots.map((bot) => (
            <button
              type="button"
              key={bot.id}
              onClick={() => onSelect(bot.id)}
              className="flex w-full items-start gap-3 p-4 text-left outline-none hover:bg-muted/40 focus-visible:ring-2 focus-visible:ring-ring"
            >
              <Bot
                aria-hidden="true"
                className="mt-0.5 size-4 shrink-0 text-muted-foreground"
              />
              <div className="min-w-0 flex-1">
                <h3 className="truncate text-sm font-medium">{bot.name}</h3>
                <p className="mt-1 line-clamp-2 text-sm text-muted-foreground">
                  {bot.instructions}
                </p>
              </div>
              <span className="text-xs text-muted-foreground">
                {bot.enabled ? "Active" : "Paused"}
              </span>
            </button>
          ))}
        </div>
      )}
    </section>
  );
}

export function CreateBot({
  api,
  open,
  onOpenChange,
  onCreated,
  defaultModel,
}: {
  api: WorkspaceApi;
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onCreated: (id: string) => void;
  defaultModel: string;
}) {
  const [model, setModel] = useState<string>();
  const [name, setName] = useState("");
  const [instructions, setInstructions] = useState("");
  const request = useRef<{
    id: string;
    name: string;
    instructions: string;
    model: string;
  } | null>(null);
  const create = useMutation({
    mutationFn: () => {
      if (
        !request.current ||
        request.current.name !== name.trim() ||
        request.current.instructions !== instructions.trim() ||
        request.current.model !== (model ?? defaultModel)
      )
        request.current = {
          id: crypto.randomUUID(),
          name: name.trim(),
          instructions: instructions.trim(),
          model: model ?? defaultModel,
        };
      return api.createBot(
        request.current.name,
        request.current.instructions,
        request.current.id,
        request.current.model,
      );
    },
    onSuccess: (bot) => {
      request.current = null;
      setName("");
      setInstructions("");
      onCreated(bot.id);
      onOpenChange(false);
    },
  });
  return (
    <Dialog
      open={open}
      onOpenChange={(next) => {
        if (!create.isPending) onOpenChange(next);
      }}
    >
      <DialogContent className="sm:max-w-lg">
        <DialogHeader>
          <DialogTitle>Create bot</DialogTitle>
          <DialogDescription>
            Give it a task and describe a useful outcome. You can refine this in
            its chat
          </DialogDescription>
        </DialogHeader>
        <form
          className="space-y-5"
          onSubmit={(event) => {
            event.preventDefault();
            if (model ?? defaultModel) create.mutate();
          }}
        >
          <div className="space-y-2">
            <label htmlFor="bot-name" className="text-sm font-medium">
              Name
            </label>
            <Input
              id="bot-name"
              autoFocus
              value={name}
              onChange={(event) => setName(event.target.value)}
              placeholder="Request reviewer"
              maxLength={100}
              required
            />
          </div>
          <div className="space-y-2">
            <label htmlFor="bot-task" className="text-sm font-medium">
              Task
            </label>
            <Textarea
              id="bot-task"
              value={instructions}
              onChange={(event) => setInstructions(event.target.value)}
              placeholder="Review new traces and tag unsuccessful user tasks using the shared task outcome annotation."
              rows={5}
              maxLength={32000}
              required
            />
          </div>
          <ModelPicker
            api={api}
            value={model ?? defaultModel}
            onChange={setModel}
            disabled={create.isPending}
          />
          <p className="text-xs leading-relaxed text-muted-foreground">
            Starts now and checks in every 10 minutes. It can use your
            workspace’s tools and publish annotations and reports. Traces stay
            read-only. Model usage is billed through LiteLLM
          </p>
          {create.error && (
            <p role="alert" className="text-sm text-destructive">
              {create.error.message}
            </p>
          )}
          <div className="flex justify-end gap-2">
            <Button
              type="button"
              variant="outline"
              disabled={create.isPending}
              onClick={() => onOpenChange(false)}
            >
              Cancel
            </Button>
            <Button
              type="submit"
              disabled={
                !name.trim() ||
                !instructions.trim() ||
                !(model ?? defaultModel) ||
                create.isPending
              }
            >
              {create.isPending && <Loader2 className="size-4 animate-spin" />}
              Create bot
            </Button>
          </div>
        </form>
      </DialogContent>
    </Dialog>
  );
}

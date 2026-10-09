import { X } from "lucide-react";

import { Button } from "../../../ui/button";

export function SignalEvidence({
  name,
  quote,
  message,
  onDismiss,
}: {
  name: string;
  quote?: string;
  message?: string;
  onDismiss: () => void;
}) {
  return (
    <section aria-label="Signal evidence" className="shrink-0 border-b bg-card px-4 py-3">
      <div className="mb-2 flex items-center justify-between gap-2">
        <h2 className="text-sm font-medium text-destructive">{name}</h2>
        <Button variant="ghost" size="icon-xs" aria-label="Dismiss signal evidence" onClick={onDismiss}>
          <X className="size-3.5" />
        </Button>
      </div>
      {quote ? (
        <>
          <p className="mb-1.5 text-xs text-muted-foreground">Flagged excerpt</p>
          <pre className="max-h-48 overflow-auto rounded-md border border-l-2 border-l-cyan-400/60 bg-muted/60 p-3 font-mono text-xs leading-5 break-words whitespace-pre-wrap">
            <mark className="rounded-sm bg-finding-quote px-0.5 text-inherit">{quote}</mark>
          </pre>
        </>
      ) : (
        <p role="status" className="text-sm text-muted-foreground">
          {message}
        </p>
      )}
    </section>
  );
}

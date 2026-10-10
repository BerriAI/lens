import { createContext, useContext, type ReactNode } from "react";
import { createPortal } from "react-dom";
import { useLensHost } from "../../../host/LensHost";
import { cn } from "../../../lib/cva.config";
import type { LensTab } from "../route";

export const LensPageActionsContext = createContext<{
  readonly target: HTMLDivElement | null;
  readonly tab: LensTab;
} | null>(null);

export function LensPageHeader({
  title,
  description,
  actions,
  navigation,
  children,
  page,
  "aria-label": label,
}: {
  readonly title: ReactNode;
  readonly description?: ReactNode;
  readonly actions?: ReactNode;
  readonly navigation?: ReactNode;
  readonly children?: ReactNode;
  readonly page?: LensTab;
  readonly "aria-label"?: string;
}) {
  const embedded = useLensHost().surface === "embedded";
  const toolbar = useContext(LensPageActionsContext);
  if (embedded && page && toolbar) {
    return (
      <>
        <h2 className="sr-only">{title}</h2>
        {toolbar.target && toolbar.tab === page && actions && createPortal(actions, toolbar.target)}
      </>
    );
  }
  return (
    <header aria-label={label} className={cn("lens-page-header", embedded && "lens-page-header-compact")}>
      {navigation && <div className={cn("text-xs text-muted-foreground", !embedded && "basis-full")}>{navigation}</div>}
      <div className={cn("min-w-0 flex-1", !embedded && "basis-full sm:basis-0")}>
        <h2 className="lens-page-title flex flex-wrap items-center gap-2">{title}</h2>
        {description && <div className="mt-1 text-sm leading-5 text-muted-foreground">{description}</div>}
      </div>
      {actions && (
        <div className={cn("flex max-w-full flex-wrap items-center gap-2", !embedded && "w-full sm:w-auto")}>
          {actions}
        </div>
      )}
      {children && <div className="min-w-0 basis-full">{children}</div>}
    </header>
  );
}

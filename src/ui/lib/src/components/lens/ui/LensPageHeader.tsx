import type { ReactNode } from "react";

export function LensPageHeader({
  title,
  description,
  actions,
  navigation,
  children,
  "aria-label": label,
}: {
  readonly title: ReactNode;
  readonly description?: ReactNode;
  readonly actions?: ReactNode;
  readonly navigation?: ReactNode;
  readonly children?: ReactNode;
  readonly "aria-label"?: string;
}) {
  return (
    <header aria-label={label} className="lens-page-header">
      {navigation && <div className="basis-full text-xs text-muted-foreground">{navigation}</div>}
      <div className="min-w-0 flex-1 basis-full sm:basis-0">
        <h2 className="lens-page-title flex flex-wrap items-center gap-2">
          {title}
        </h2>
        {description && <div className="mt-1 text-sm leading-5 text-muted-foreground">{description}</div>}
      </div>
      {actions && <div className="flex w-full max-w-full flex-wrap items-center gap-2 sm:w-auto">{actions}</div>}
      {children && <div className="min-w-0 basis-full">{children}</div>}
    </header>
  );
}

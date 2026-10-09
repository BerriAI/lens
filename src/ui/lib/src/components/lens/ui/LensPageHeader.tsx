import type { ReactNode } from "react";

export function LensPageHeader({
  section,
  title,
  description,
  actions,
}: {
  readonly section: string;
  readonly title: ReactNode;
  readonly description?: string;
  readonly actions?: ReactNode;
}) {
  return (
    <header className="lens-page-header">
      <div className="relative min-w-0">
        <p className="lens-section-label mb-2 flex items-center gap-2">
          <span aria-hidden="true" className="lens-spectrum">
            <i />
            <i />
            <i />
            <i />
          </span>
          {section}
        </p>
        <h2 className="flex items-center gap-2.5 text-xl font-medium tracking-tight sm:text-2xl">{title}</h2>
        {description && <p className="mt-1 text-xs leading-5 text-muted-foreground">{description}</p>}
      </div>
      {actions && <div className="relative flex flex-wrap items-center gap-2">{actions}</div>}
    </header>
  );
}

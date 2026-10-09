import type { ComponentProps, ReactNode } from "react";
import { cn } from "../../../lib/cva.config";

export type SettingsSectionProps = ComponentProps<"section"> & {
  heading: string;
  description: string;
  icon?: ReactNode;
};

export function SettingsSection({ heading, description, icon, className, children, ...props }: SettingsSectionProps) {
  return (
    <section
      data-slot="settings-section"
      aria-label={heading}
      {...props}
      className={cn("grid gap-4 py-6 first:pt-0 last:pb-0 md:grid-cols-[200px_minmax(0,1fr)] md:gap-8", className)}
    >
      <div className="space-y-2">
        {icon && (
          <span className="mb-3 flex size-9 items-center justify-center rounded-lg border border-[var(--lens-brand)]/20 bg-[var(--lens-brand)]/10 text-[var(--lens-brand)]">
            {icon}
          </span>
        )}
        <h2 className="lens-section-label text-xs font-semibold">{heading}</h2>
        <p className="text-xs leading-5 text-muted-foreground">{description}</p>
      </div>
      <div className="min-w-0 space-y-3">{children}</div>
    </section>
  );
}

export type SettingsCardProps = ComponentProps<"div">;

export function SettingsCard({ className, ...props }: SettingsCardProps) {
  return (
    <div
      data-slot="settings-card"
      {...props}
      className={cn(
        "lens-panel rounded-xl border border-border p-4 [&_input]:bg-background [&_textarea]:bg-background",
        className,
      )}
    />
  );
}

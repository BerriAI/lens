export function DetailGroup({ title, children }: { title: string; children: React.ReactNode }) {
  return (
    <section aria-label={title} className="flex flex-col gap-2">
      <h3 className="lens-section-label">{title}</h3>
      <div className="rounded-lg border border-border bg-card px-3 py-1.5">{children}</div>
    </section>
  );
}

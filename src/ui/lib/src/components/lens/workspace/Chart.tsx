import { useEffect, useRef, useState } from "react";
import { useTheme } from "next-themes";
import type { View } from "vega";
import type { TopLevelSpec } from "vega-lite";

export const CHART_MEDIA_TYPE = "application/vnd.vegalite.v6+json";

export function chartSpec(content: string, compact: boolean): TopLevelSpec {
  const spec: unknown = JSON.parse(content);
  if (!spec || typeof spec !== "object" || Array.isArray(spec))
    throw new Error("Invalid chart");
  const rejectLinks = (value: unknown): void => {
    if (!value || typeof value !== "object") return;
    for (const [key, child] of Object.entries(value)) {
      if (key === "url" || key === "href")
        throw new Error("Charts must contain their data and cannot open links");
      rejectLinks(child);
    }
  };
  rejectLinks(spec);
  const {
    config: _config,
    background: _background,
    title: _title,
    ...contentSpec
  } = spec as Record<string, unknown>;
  return {
    ...contentSpec,
    width: "container",
    height: compact ? 170 : 300,
    autosize: { type: "fit", contains: "padding", resize: true },
  } as TopLevelSpec;
}

export function Chart({
  content,
  compact = false,
  label,
}: {
  content: string;
  compact?: boolean;
  label: string;
}) {
  const container = useRef<HTMLDivElement>(null);
  const { resolvedTheme } = useTheme();
  const [error, setError] = useState(false);
  useEffect(() => {
    const element = container.current;
    if (!element) return;
    let cancelled = false;
    let view: View | undefined;
    let observer: ResizeObserver | undefined;
    setError(false);
    const render = async () => {
      const [vega, { compile }, { expressionInterpreter }] = await Promise.all([
        import("vega"),
        import("vega-lite"),
        import("vega-interpreter"),
      ]);
      if (cancelled) return;
      const style = getComputedStyle(element);
      const context = document.createElement("canvas").getContext("2d");
      const color = (token: string) => {
        if (!context) return style.color;
        context.fillStyle = style.getPropertyValue(token).trim();
        context.fillRect(0, 0, 1, 1);
        const [r, g, b] = context.getImageData(0, 0, 1, 1).data;
        return `rgb(${r}, ${g}, ${b})`;
      };
      const ink = color("--muted-foreground");
      const primary = color("--primary");
      const compiled = compile(chartSpec(content, compact), {
        config: {
          background: "transparent",
          font: style.fontFamily,
          view: { stroke: null },
          axis: {
            labelColor: ink,
            titleColor: ink,
            labelFontSize: 11,
            titleFontSize: 11,
            titleFontWeight: "normal",
            gridColor: color("--border"),
            domain: false,
            ticks: false,
            labelPadding: 8,
            titlePadding: 10,
          },
          legend: {
            labelColor: ink,
            titleColor: ink,
            labelFontSize: 11,
            titleFontSize: 11,
            orient: "bottom",
          },
          mark: { color: primary, tooltip: true },
          bar: { cornerRadiusEnd: 3 },
          range: {
            category: [
              primary,
              color("--chart-2"),
              color("--chart-3"),
              color("--chart-4"),
              color("--chart-5"),
            ],
          },
        },
      }).spec;
      const blocked = async (): Promise<never> => {
        throw new Error("External chart resources are disabled");
      };
      view = new vega.View(vega.parse(compiled, undefined, { ast: true }), {
        expr: expressionInterpreter,
        renderer: "svg",
        container: element,
        loader: {
          load: blocked,
          sanitize: blocked,
          http: blocked,
          file: blocked,
        },
        logLevel: vega.None,
      });
      await view.runAsync();
      if (cancelled) {
        view.finalize();
        return;
      }
      observer = new ResizeObserver(() => {
        if (!cancelled)
          void view
            ?.resize()
            .runAsync()
            .catch(() => setError(true));
      });
      observer.observe(element);
    };
    void render().catch(() => {
      if (!cancelled) setError(true);
    });
    return () => {
      cancelled = true;
      observer?.disconnect();
      view?.finalize();
      element.replaceChildren();
    };
  }, [content, compact, resolvedTheme]);
  return error ? (
    <p role="alert" className="py-3 text-xs text-muted-foreground">
      This chart could not be displayed. Its data is available in the attachment
    </p>
  ) : (
    <div
      ref={container}
      role="img"
      aria-label={label}
      className="w-full min-w-0 overflow-x-auto [&_svg]:block"
    />
  );
}

# Lens design language

Lens is one workspace for connecting agents, inspecting traces, investigating findings, and testing changes. Keep the navigation, typography, spacing, and controls familiar as users move between these tasks

## Shared page structure

Use [LensPageHeader](ui/LensPageHeader.tsx) for page titles, descriptions, navigation back to a collection, and page actions. Use `lens-page-body` for content padding. The workspace owns the outer frame, so pages must not add another page frame or another set of horizontal padding around the header

| Element                              | Treatment                                                                    |
| ------------------------------------ | ---------------------------------------------------------------------------- |
| Page title                           | `lens-page-title`: 20px, 28px line height, weight 600                        |
| Page description                     | 14px, 20px line height, muted; explain the task in one sentence when helpful |
| Section heading                      | `lens-section-label`: 14px, 20px line height, weight 500, sentence case      |
| Table heading and secondary metadata | 12px, sentence case, muted                                                   |
| Body and controls                    | 14px; dense trace and case tables can use 12px                               |
| Page gutters                         | `--lens-page-gutter`: 24px, reduced to 16px below 640px                      |
| Header padding                       | 20px vertically; shared page gutters horizontally                            |
| Body padding                         | 24px vertically; shared page gutters horizontally                            |
| Related controls                     | 8px gap                                                                      |
| Related content                      | 16px gap                                                                     |
| Separate sections                    | 24px gap                                                                     |
| Panels                               | `lens-panel`: 1px border, 12px corners, card surface                         |

Do not add an eyebrow, numbered category, uppercase heading, or decorative icon above a page title. A breadcrumb earns its place when it takes the user back to the collection. Put it in the header's `navigation` slot. Keep IDs and copy controls in `actions` so the title stays readable and accessible

Use the UI font for prose, labels, names, dates, counts, and controls. Use `tabular-nums` to align numbers. Keep monospace for code, raw identifiers, model names, and technical payloads

## Personality and color

Keep the dotted background. It belongs in the shared page header, the workspace backdrop, and empty inspection canvases. Keep the dots subtle and theme-aware using `--lens-grid`; use plain surfaces behind tables, forms, prose, and trace content. Do not give individual pages their own pattern or gradient

Use the existing Lens brand blue for the primary action, active navigation, and selection. Use neutral count badges and neutral feature icons. Reserve success, warning, and destructive colors for actual state. Charts and trace step types may use the existing data palette. Never rely on color alone to explain a state

Use `bg-card`, `text-foreground`, `text-muted-foreground`, `border-border`, and the Lens tokens in [styles.css](../../../styles.css). Avoid page-specific colors and shadows. Check light and dark mode with the same content

## Layout follows the task

Collection pages use the shared header followed by a padded content area. Agents includes search and a directory table. Datasets and Evals keep their tables scrollable within that area. Counts, filters, and primary actions stay close to the content they affect

Findings keeps a list beside an inspection pane. Its analysis status is a compact toolbar below the page header. On narrow screens, selecting a finding shows the detail view with a way back to the list

Traces is a dense working surface. Keep its search, time range, activity chart, and table immediately below the workspace navigation. It does not need a second large title repeating “Traces”. Trace drawers and span panels keep compact 14px titles and controls so recorded content has room

Dataset and eval detail pages use the same page header as their collections. Place revision controls, exports, and save actions in the action area. Put evaluation status and run metadata below the title. Case inspectors and trace inspectors use compact local headers

Home and setup retain a readable content width, complete copyable instructions, and a visible connection state. Settings retains its label-and-control columns, which stack on narrow screens. Forms may use a narrower content column without moving their page header away from the shared gutter

Inside the gateway, use one compact top bar for tabs, agent selection, page actions, and workspace controls. Collection headers pass their `page` to `LensPageHeader`, which keeps the heading accessible and places actions in that bar. Do not repeat the collection title or description below it. Detail views keep a compact contextual toolbar for their title, back navigation, metadata, and actions. Embedded content uses 16px gutters and padding; tabs scroll and controls wrap at narrow widths

## Actions, state, and recovery

Use the shared Button, Input, Select, Tabs, and state primitives. Keep one primary next action for the current task. Use outline or ghost styling for secondary actions. Controls wrap on narrow screens instead of pushing the page beyond the viewport; wide data tables scroll inside their panel

Preserve setup state, permissions, retries, and direct navigation to recorded results. Existing configuration should be reused. Keep optional settings at the point of need. “Service ready”, “sample data”, and “a real trace received” remain distinct states

Empty states explain what is missing and the next useful action. Loading states occupy the space of the pending content. Errors name the failed operation and retain a retry or a route back. Do not change the product's state or hide required actions to make a screenshot cleaner

## Verify a change

Capture the same content and state before and after at desktop and narrow widths, in light and dark mode. Label fixtures and demo data in the evidence. Inspect headers, gutters, wrapping, scrolling, focus, dropdowns, and error states in the browser

Run the UI typecheck, production build, and the specific existing component or integration tests that cover the affected actions. Add a behavioral test when a change introduces or fixes behavior; do not assert implementation structure or pixel classes in unit tests

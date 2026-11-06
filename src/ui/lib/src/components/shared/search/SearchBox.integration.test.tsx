import { fireEvent, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { Hash } from "lucide-react";
import { expect, it, vi } from "vitest";
import { SearchBox } from "./SearchBox";
import { EQUALITY_ONLY, type QueryLanguage } from "./language";
import type { SearchQuery } from "./searchQuery";

const language: QueryLanguage<"tag"> = {
  fields: { tag: { icon: Hash, group: "Run attributes", suggestValues: true } },
  ops: EQUALITY_ONLY,
};
const values = { useValues: () => ({ values: ["diagram"], loading: false }) };
const supports = (query: SearchQuery<"tag">) => query.filters.length === 0;

it("does not compute or display a copied command for an unsupported query", () => {
  const command = vi.fn(() => "curl broader-query");
  render(
    <SearchBox.Root
      language={language}
      values={values}
      value="tag:diagram"
      onValueChange={vi.fn()}
      label="Search"
    >
      <SearchBox.Input placeholder="Search" />
      <SearchBox.CopyCommand
        title="Copy query"
        command={command}
        supports={supports}
      />
    </SearchBox.Root>,
  );
  expect(
    screen.queryByRole("button", { name: "Copy as curl" }),
  ).not.toBeInTheDocument();
  expect(command).not.toHaveBeenCalled();
});

it("uses current editor clauses before the parent echoes a debounced edit", async () => {
  const user = userEvent.setup();
  const command = vi.fn(() => "curl broader-query");
  render(
    <SearchBox.Root
      language={language}
      values={values}
      value=""
      onValueChange={vi.fn()}
      label="Search"
    >
      <SearchBox.Input placeholder="Search" />
      <SearchBox.CopyCommand
        title="Copy query"
        command={command}
        supports={supports}
      />
    </SearchBox.Root>,
  );
  expect(screen.getByRole("button", { name: "Copy as curl" })).toBeVisible();
  const input = screen.getByRole("combobox", { name: "Search" });
  await user.click(input);
  await user.keyboard("tag:diagram");
  expect(
    screen.queryByRole("button", { name: "Copy as curl" }),
  ).not.toBeInTheDocument();
  expect(command).not.toHaveBeenCalledWith(
    expect.objectContaining({
      filters: expect.arrayContaining([
        expect.objectContaining({ field: "tag" }),
      ]),
    }),
  );
});

it("preserves whitespace inside a pasted quoted tag", async () => {
  const user = userEvent.setup();
  const supported = vi.fn(() => false);
  render(
    <SearchBox.Root
      language={language}
      values={values}
      value=""
      onValueChange={vi.fn()}
      label="Search"
    >
      <SearchBox.Input placeholder="Search" />
      <SearchBox.CopyCommand
        title="Copy query"
        command={() => "unused"}
        supports={supported}
      />
    </SearchBox.Root>,
  );
  await user.click(screen.getByRole("combobox", { name: "Search" }));
  fireEvent.paste(screen.getByRole("combobox", { name: "Search" }), {
    clipboardData: {
      getData: (format: string) =>
        format === "text/plain" ? 'tag:"team  platform"' : "",
      files: [],
    },
  });
  expect(screen.getByRole("combobox", { name: "Search" })).toHaveTextContent(
    'tag:"team  platform"',
    { normalizeWhitespace: false },
  );
  expect(supported).toHaveBeenLastCalledWith({
    text: [],
    filters: [{ field: "tag", op: "eq", value: "team  platform" }],
  });
});

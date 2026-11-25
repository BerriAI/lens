import { screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { ThemeProvider } from "next-themes";
import { beforeEach, expect, it, vi } from "vitest";
import {
  renderWithProviders,
  testQueryClient,
} from "../../../tests/test-utils";
import {
  admin,
  bot,
  workspaceNetwork,
} from "../../../tests/lens-workspace-utils";
import { LensWorkspace } from "./LensWorkspace";
import { LensHostProvider } from "../../host/LensHost";

beforeEach(() => {
  testQueryClient.clear();
  window.localStorage.clear();
  window.sessionStorage.clear();
  Element.prototype.scrollIntoView = vi.fn();
  vi.stubGlobal("fetch", workspaceNetwork());
});

it("switches between the main chats and the selected bot's chats", async () => {
  const user = userEvent.setup();
  const onUrlUpdate = vi.fn();
  renderWithProviders(<LensWorkspace {...admin} />, { onUrlUpdate });
  expect(
    await screen.findByRole("button", { name: "Investigate failed requests" }),
  ).toBeVisible();
  const nav = within(screen.getByRole("navigation", { name: "Workspace" }));
  expect(
    nav
      .getAllByRole("button")
      .map((button) => button.textContent)
      .filter(Boolean),
  ).toEqual(["Home", "New Session", "Bots", "Traces"]);
  await user.click(nav.getByRole("button", { name: "Bots" }));
  await user.click(screen.getByRole("button", { name: bot.name, exact: true }));
  expect(await screen.findByRole("button", { name: "Bot Chat" })).toBeVisible();
  expect(
    screen.getByRole("textbox", { name: "Message Request reviewer" }),
  ).toBeEnabled();
  expect(
    screen.queryByRole("button", { name: "Investigate failed requests" }),
  ).not.toBeInTheDocument();
  await user.click(nav.getByRole("button", { name: "New Session" }));
  expect(
    await screen.findByRole("textbox", { name: "Message Lens" }),
  ).toBeEnabled();
  await waitFor(() =>
    expect(onUrlUpdate.mock.lastCall?.[0].searchParams.get("tab")).toBe(
      "new_session",
    ),
  );
});

it("keeps navigation and footer links accessible when collapsed", async () => {
  const user = userEvent.setup();
  renderWithProviders(<LensWorkspace {...admin} />);
  await user.click(screen.getByRole("button", { name: "Collapse navigation" }));
  expect(
    screen.getByRole("button", { name: "Bots", exact: true }),
  ).toHaveAttribute("title", "Bots");
  expect(screen.getByRole("link", { name: "Documentation" })).toBeVisible();
  expect(screen.getByRole("link", { name: "Book a call" })).toHaveAttribute(
    "rel",
    "noopener noreferrer",
  );
  await user.click(screen.getByRole("button", { name: "Bots", exact: true }));
  expect(
    await screen.findByRole("heading", { name: "Give a bot an ongoing task" }),
  ).toBeVisible();
});

it("should show the signed in user in the sidebar and log out from its menu", async () => {
  const user = userEvent.setup();
  const signOut = vi.fn();
  const view = renderWithProviders(
    <LensHostProvider
      host={{
        surface: "standalone",
        user: {
          name: "viewer@example.com",
          role: "proxy_admin_viewer",
          signOut,
        },
      }}
    >
      <LensWorkspace accessToken="" userRole="proxy_admin_viewer" readOnly />
    </LensHostProvider>,
  );
  const sidebar = within(
    await screen.findByRole("complementary", { name: "Lens navigation" }),
  );
  const account = sidebar.getByRole("button", {
    name: "Account: viewer@example.com, Admin Viewer",
  });
  expect(account).toHaveTextContent("VI");
  expect(within(account).getByText("viewer@example.com")).toBeVisible();
  expect(within(account).getByText("Admin Viewer")).toBeVisible();
  expect(
    screen.queryByRole("menuitem", { name: "Logout" }),
  ).not.toBeInTheDocument();
  await user.click(account);
  await user.click(await screen.findByRole("menuitem", { name: "Logout" }));
  expect(signOut).toHaveBeenCalledOnce();

  await user.click(
    sidebar.getByRole("button", { name: "Collapse navigation" }),
  );
  const collapsed = sidebar.getByRole("button", {
    name: "Account: viewer@example.com, Admin Viewer",
  });
  expect(
    within(collapsed).queryByText("viewer@example.com"),
  ).not.toBeInTheDocument();
  await user.click(collapsed);
  await user.click(await screen.findByRole("menuitem", { name: "Logout" }));
  expect(signOut).toHaveBeenCalledTimes(2);

  view.rerender(
    <LensHostProvider host={{ surface: "standalone" }}>
      <LensWorkspace accessToken="" userRole="proxy_admin_viewer" readOnly />
    </LensHostProvider>,
  );
  expect(sidebar.getByRole("link", { name: "Documentation" })).toBeVisible();
  expect(
    sidebar.queryByRole("button", { name: /^Account:/ }),
  ).not.toBeInTheDocument();
});

it("should switch color themes from the sidebar and restore the saved preference after reopening", async () => {
  const user = userEvent.setup();
  const initialClassName = document.documentElement.className;
  const initialColorScheme = document.documentElement.style.colorScheme;
  const storageKey = "lens-theme-test";
  const workspace = (
    <ThemeProvider
      attribute="class"
      defaultTheme="light"
      storageKey={storageKey}
    >
      <LensWorkspace
        accessToken="live-token"
        userRole="Admin"
        readOnly={false}
      />
    </ThemeProvider>
  );

  try {
    const first = renderWithProviders(workspace, {
      searchParams: "?tab=home",
    });
    expect(document.documentElement).toHaveClass("light");
    expect(screen.getByRole("button", { name: "Light mode" })).toHaveAttribute(
      "aria-pressed",
      "true",
    );
    await user.click(screen.getByRole("button", { name: "Dark mode" }));
    expect(document.documentElement).toHaveClass("dark");
    expect(screen.getByRole("button", { name: "Dark mode" })).toHaveAttribute(
      "aria-pressed",
      "true",
    );
    expect(screen.getByRole("button", { name: "Light mode" })).toHaveAttribute(
      "aria-pressed",
      "false",
    );
    expect(window.localStorage.getItem(storageKey)).toBe("dark");

    first.unmount();
    document.documentElement.classList.remove("dark");
    const reopened = renderWithProviders(workspace, {
      searchParams: "?tab=home",
    });
    expect(document.documentElement).toHaveClass("dark");
    expect(screen.getByRole("button", { name: "Dark mode" })).toHaveAttribute(
      "aria-pressed",
      "true",
    );
    await user.click(screen.getByRole("button", { name: "Light mode" }));
    expect(document.documentElement).toHaveClass("light");
    expect(document.documentElement).not.toHaveClass("dark");
    expect(screen.getByRole("button", { name: "Light mode" })).toHaveAttribute(
      "aria-pressed",
      "true",
    );
    expect(screen.getByRole("button", { name: "Dark mode" })).toHaveAttribute(
      "aria-pressed",
      "false",
    );
    expect(window.localStorage.getItem(storageKey)).toBe("light");

    await user.click(
      screen.getByRole("button", { name: "Collapse navigation" }),
    );
    await user.click(
      screen.getByRole("button", { name: "Toggle color theme" }),
    );
    expect(document.documentElement).toHaveClass("dark");
    expect(window.localStorage.getItem(storageKey)).toBe("dark");
    await user.click(screen.getByRole("button", { name: "Expand navigation" }));
    expect(screen.getByRole("button", { name: "Dark mode" })).toHaveAttribute(
      "aria-pressed",
      "true",
    );
    reopened.unmount();
  } finally {
    document.documentElement.className = initialClassName;
    document.documentElement.style.colorScheme = initialColorScheme;
    window.localStorage.removeItem(storageKey);
  }
});

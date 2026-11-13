import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { useState } from "react";
import { expect, it } from "vitest";
import { SegmentedControl } from "./SegmentedControl";

const OPTIONS = [
  { value: "runs", label: "Runs" },
  { value: "spend", label: "Spend" },
  { value: "latency", label: "Latency" },
] as const;

function Harness() {
  const [value, setValue] = useState<(typeof OPTIONS)[number]["value"]>("spend");
  return (
    <>
      <button type="button">Before</button>
      <SegmentedControl label="Rank by" options={OPTIONS} value={value} onChange={setValue} />
    </>
  );
}

const radio = (name: string) => screen.getByRole("radio", { name });

it("tabs only to the checked option and moves the choice with the arrow keys", async () => {
  const user = userEvent.setup();
  render(<Harness />);

  await user.click(screen.getByRole("button", { name: "Before" }));
  await user.tab();
  expect(radio("Spend")).toHaveFocus();

  await user.keyboard("{ArrowRight}");
  expect(radio("Latency")).toHaveFocus();
  expect(radio("Latency")).toHaveAttribute("aria-checked", "true");
  expect(radio("Spend")).toHaveAttribute("aria-checked", "false");

  await user.keyboard("{ArrowRight}");
  expect(radio("Runs")).toHaveFocus();
  expect(radio("Runs")).toHaveAttribute("aria-checked", "true");

  await user.keyboard("{ArrowLeft}");
  expect(radio("Latency")).toHaveFocus();
  expect(radio("Latency")).toHaveAttribute("aria-checked", "true");

  await user.tab({ shift: true });
  expect(screen.getByRole("button", { name: "Before" })).toHaveFocus();
});

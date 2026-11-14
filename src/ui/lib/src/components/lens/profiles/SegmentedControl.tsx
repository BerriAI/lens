"use client";

import { Radio } from "@base-ui/react/radio";
import { RadioGroup } from "@base-ui/react/radio-group";

export function SegmentedControl<T extends string>({
  label,
  options,
  value,
  onChange,
}: {
  label: string;
  options: readonly { value: T; label: string }[];
  value: T;
  onChange: (value: T) => void;
}) {
  return (
    <RadioGroup<T>
      aria-label={label}
      value={value}
      onValueChange={onChange}
      className="flex w-fit items-center rounded-md bg-muted p-0.5"
    >
      {options.map((option) => (
        <Radio.Root
          key={option.value}
          value={option.value}
          className="h-6 cursor-pointer rounded-sm px-2.5 text-xs leading-6 whitespace-nowrap text-muted-foreground transition-colors duration-150 hover:text-foreground focus-visible:ring-[3px] focus-visible:ring-ring/50 focus-visible:outline-none motion-reduce:transition-none data-checked:bg-background data-checked:font-medium data-checked:text-foreground data-checked:shadow-xs"
        >
          {option.label}
        </Radio.Root>
      ))}
    </RadioGroup>
  );
}

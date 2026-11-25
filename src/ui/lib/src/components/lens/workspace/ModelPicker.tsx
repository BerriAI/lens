import { useContext, useEffect } from "react";
import { useQuery } from "@tanstack/react-query";
import {
  Combobox,
  ComboboxContent,
  ComboboxItem,
  ComboboxTrigger,
  ComboboxValue,
  ComboboxInput,
  ComboboxList,
  ComboboxEmpty,
} from "../../ui/combobox";
import { Button } from "../../ui/button";
import type { WorkspaceApi } from "./Chat";
import { WorkspaceScope } from "./Results";

export function ModelPicker({
  api,
  value,
  onChange,
  disabled,
}: {
  api: WorkspaceApi;
  value: string;
  onChange: (model: string) => void;
  disabled?: boolean;
}) {
  const scope = useContext(WorkspaceScope);
  const models = useQuery({
    queryKey: ["agent-workspace", scope, "models"],
    queryFn: api.models,
    enabled: !disabled,
    staleTime: 300000,
  });
  useEffect(() => {
    if (models.data && !disabled && !models.data.includes(value))
      onChange(models.data[0] ?? "");
  }, [models.data, value, onChange, disabled]);
  const options = models.data ?? (value ? [value] : []);
  if (models.data?.length === 0)
    return (
      <p className="text-xs text-muted-foreground">
        Add a model in LiteLLM to start chatting{" "}
        <Button
          type="button"
          variant="link"
          size="sm"
          disabled={models.isFetching}
          onClick={() => void models.refetch()}
        >
          Refresh models
        </Button>
      </p>
    );
  return (
    <div className="min-w-0">
      <Combobox
        items={options}
        value={value}
        onValueChange={(model) => {
          if (model) onChange(model);
        }}
        disabled={disabled || models.isPending}
      >
        <ComboboxTrigger
          aria-label="Model"
          render={
            <Button
              type="button"
              variant="ghost"
              size="sm"
              className="max-w-72 px-1 text-xs text-muted-foreground"
            />
          }
        >
          <span className="truncate">
            <ComboboxValue
              placeholder={
                models.isPending ? "Loading models…" : "Choose model"
              }
            />
          </span>
        </ComboboxTrigger>
        <ComboboxContent className="w-80" align="start">
          <ComboboxInput
            aria-label="Search models"
            placeholder="Search models…"
            showTrigger={false}
          />
          <ComboboxEmpty>No matching models</ComboboxEmpty>
          <ComboboxList>
            {(model: string) => (
              <ComboboxItem key={model} value={model}>
                {model}
              </ComboboxItem>
            )}
          </ComboboxList>
        </ComboboxContent>
      </Combobox>
      {models.error && (
        <p role="alert" className="text-xs text-destructive">
          Could not load models{" "}
          <Button
            variant="link"
            size="sm"
            onClick={() => void models.refetch()}
          >
            Retry
          </Button>
        </p>
      )}
    </div>
  );
}

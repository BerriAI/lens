"use client";

import { Plug } from "lucide-react";
import { Button } from "../../ui/button";
import { useLensRoute, useConnectorRoute } from "../route";
import { SettingsSection } from "./SettingsSection";

export function McpConnection() {
  const lens = useLensRoute();
  const connector = useConnectorRoute();
  return (
    <SettingsSection
      heading="Lens MCP"
      icon={<Plug aria-hidden="true" className="size-4" />}
      description="Set up your agent, manage access, and view usage in Connectors"
    >
      <Button
        variant="outline"
        size="sm"
        onClick={() => {
          lens.setTab("connectors");
          connector.open();
        }}
      >
        Open Connectors
      </Button>
    </SettingsSection>
  );
}

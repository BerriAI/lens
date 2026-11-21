import { Suspense } from "react";
import { EmbeddedWorkspace } from "./embedded";
import { Workspace } from "./workspace";

export default function Page() {
  return (
    <Suspense>
      {process.env.NEXT_PUBLIC_LENS_SURFACE === "embedded" ? (
        <EmbeddedWorkspace />
      ) : (
        <Workspace />
      )}
    </Suspense>
  );
}

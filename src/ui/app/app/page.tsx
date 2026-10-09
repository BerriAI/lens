import { Suspense } from "react";
import { Workspace } from "./workspace";

export default function Page() {
  return (
    <Suspense>
      <Workspace />
    </Suspense>
  );
}

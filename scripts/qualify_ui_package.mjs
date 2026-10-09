import { execFileSync } from "node:child_process";
import { cp, mkdtemp, readFile, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const destination = await mkdtemp(join(tmpdir(), "lens-ui-package-"));
const packed = JSON.parse(
  execFileSync(
    "npm",
    [
      "pack",
      "--workspace",
      "@litellm/lens-ui",
      "--pack-destination",
      destination,
      "--json",
    ],
    { cwd: root, encoding: "utf8" },
  ),
)[0];
const workspace = JSON.parse(
  await readFile(join(root, "package.json"), "utf8"),
);
const web = JSON.parse(
  await readFile(join(root, "src/ui/app/package.json"), "utf8"),
);
const manifest = {
  ...web,
  name: "lens-ui-package-qualification",
  dependencies: {
    ...web.dependencies,
    "@litellm/lens-ui": `file:./${packed.filename}`,
  },
  devDependencies: workspace.devDependencies,
  overrides: workspace.overrides,
};
await writeFile(
  join(destination, "package.json"),
  JSON.stringify(manifest, null, 2),
);
for (const name of [
  "app",
  "next.config.mjs",
  "postcss.config.mjs",
  "tsconfig.json",
  "next-env.d.ts",
]) {
  await cp(join(root, "src/ui/app", name), join(destination, name), {
    recursive: true,
  });
}
console.log(`Qualifying the packed UI in ${destination}`);
execFileSync("npm", ["install", "--ignore-scripts"], {
  cwd: destination,
  stdio: "inherit",
});
execFileSync("npm", ["run", "build"], { cwd: destination, stdio: "inherit" });
console.log(
  JSON.stringify(
    {
      package: packed.filename,
      integrity: packed.integrity,
      entries: packed.entryCount,
      directory: destination,
      staticOutput: join(destination, "out"),
    },
    null,
    2,
  ),
);

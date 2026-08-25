import { existsSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { spawnSync } from "node:child_process";

const scriptDirectory = dirname(fileURLToPath(import.meta.url));
const appRoot = resolve(scriptDirectory, "..");
const authFrontend = resolve(appRoot, "..", "orcestr-auth", "frontend");
const authOnly = process.argv.includes("--auth-only");
const npmCli = process.env.npm_execpath;

if (!existsSync(join(authFrontend, "package-lock.json"))) {
  throw new Error(
    `Expected the sibling orcestr-auth repository at ${resolve(appRoot, "..", "orcestr-auth")}`,
  );
}
if (!npmCli || !existsSync(npmCli)) {
  throw new Error("Run this installer through `npm run deps:install` or `npm run auth:local`");
}

function run(arguments_, cwd) {
  const result = spawnSync(process.execPath, [npmCli, ...arguments_], {
    cwd,
    stdio: "inherit",
    shell: false,
  });
  if (result.error) throw result.error;
  if (result.status !== 0) {
    throw new Error(`npm ${arguments_.join(" ")} failed with exit code ${result.status}`);
  }
}

run(["ci"], authFrontend);
run(
  [
    "run",
    "build",
    "--workspace",
    "@orcestr/auth-core",
    "--workspace",
    "@orcestr/auth-react",
    "--workspace",
    "@orcestr/auth-forms",
  ],
  authFrontend,
);

if (!authOnly) {
  run(["ci"], appRoot);
}

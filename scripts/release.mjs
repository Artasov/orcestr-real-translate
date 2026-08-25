import { execFileSync, spawnSync } from "node:child_process";
import { readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import process from "node:process";
import { fileURLToPath, pathToFileURL } from "node:url";

const scriptDirectory = dirname(fileURLToPath(import.meta.url));
const repositoryRoot = resolve(scriptDirectory, "..");
const allowedBumps = new Set(["patch", "minor", "major"]);
const versionFiles = [
  "package.json",
  "package-lock.json",
  "src-tauri/Cargo.toml",
  "src-tauri/Cargo.lock",
  "src-tauri/tauri.conf.json",
];

export function nextStableVersion(currentVersion, bump) {
  if (!allowedBumps.has(bump)) {
    throw new Error(`Release type must be patch, minor or major; received ${JSON.stringify(bump)}`);
  }
  const match = /^(\d+)\.(\d+)\.(\d+)$/.exec(String(currentVersion));
  if (!match) {
    throw new Error(`Release shortcuts require a stable semantic version; received ${JSON.stringify(currentVersion)}`);
  }
  const [, major, minor, patch] = match.map(Number);
  if (bump === "major") return `${major + 1}.0.0`;
  if (bump === "minor") return `${major}.${minor + 1}.0`;
  return `${major}.${minor}.${patch + 1}`;
}

function execute(executable, arguments_, options = {}) {
  return execFileSync(executable, arguments_, {
    cwd: repositoryRoot,
    encoding: "utf8",
    env: process.env,
    stdio: options.capture === false ? "inherit" : ["ignore", "pipe", "pipe"],
  });
}

function output(executable, arguments_) {
  return execute(executable, arguments_).trim();
}

function status(executable, arguments_) {
  return spawnSync(executable, arguments_, {
    cwd: repositoryRoot,
    env: process.env,
    stdio: "ignore",
  }).status;
}

function git(arguments_, options = {}) {
  return options.capture === false
    ? execute("git", arguments_, { capture: false })
    : output("git", arguments_);
}

function npmRun(script) {
  const npmCli = process.env.npm_execpath;
  if (npmCli) {
    execute(process.execPath, [npmCli, "run", script], { capture: false });
    return;
  }
  const npm = process.platform === "win32" ? "npm.cmd" : "npm";
  execute(npm, ["run", script], { capture: false });
}

function requireCleanWorktree() {
  const changes = git(["status", "--porcelain=v1", "--untracked-files=all"]);
  if (changes) {
    throw new Error(
      `Release aborted: commit or stash every change first.\n${changes.split("\n").slice(0, 20).join("\n")}`,
    );
  }
}

function requireMissingRemoteRef(remote, reference, label) {
  const result = spawnSync(
    "git",
    ["ls-remote", "--exit-code", remote, reference],
    { cwd: repositoryRoot, encoding: "utf8", stdio: ["ignore", "pipe", "pipe"] },
  );
  if (result.status === 0) throw new Error(`Release aborted: ${label} already exists on ${remote}.`);
  if (result.status !== 2) {
    throw new Error(`Release aborted: could not inspect ${remote}.\n${String(result.stderr).trim()}`);
  }
}

function runRelease(bump) {
  if (!allowedBumps.has(bump)) {
    throw new Error("Usage: node scripts/release.mjs <patch|minor|major>");
  }

  const mainBranch = process.env.RELEASE_BRANCH || "main";
  const currentBranch = git(["branch", "--show-current"]);
  if (currentBranch !== mainBranch) {
    throw new Error(`Release aborted: switch to ${mainBranch}; current branch is ${currentBranch || "<detached>"}.`);
  }
  requireCleanWorktree();

  const remote = process.env.RELEASE_REMOTE || git(["config", "--get", `branch.${mainBranch}.remote`]) || "origin";
  console.log(`[release] Fetching ${remote}/${mainBranch} and tags...`);
  git(
    ["fetch", "--prune", "--tags", remote, `refs/heads/${mainBranch}:refs/remotes/${remote}/${mainBranch}`],
    { capture: false },
  );
  const localMain = git(["rev-parse", "HEAD"]);
  const remoteMain = git(["rev-parse", `refs/remotes/${remote}/${mainBranch}`]);
  if (localMain !== remoteMain) {
    throw new Error(`Release aborted: local ${mainBranch} must exactly match ${remote}/${mainBranch}.`);
  }
  if (status("gh", ["auth", "status"]) !== 0) {
    throw new Error("Release aborted: GitHub CLI is not authenticated.");
  }

  const packageJson = JSON.parse(readFileSync(resolve(repositoryRoot, "package.json"), "utf8"));
  const currentVersion = String(packageJson.version ?? "");
  const nextVersion = nextStableVersion(currentVersion, bump);
  const tag = `v${nextVersion}`;
  const releaseBranch = `release/${tag}`;

  if (status("git", ["show-ref", "--verify", "--quiet", `refs/tags/${tag}`]) === 0) {
    throw new Error(`Release aborted: local tag ${tag} already exists.`);
  }
  if (status("git", ["show-ref", "--verify", "--quiet", `refs/heads/${releaseBranch}`]) === 0) {
    throw new Error(`Release aborted: local branch ${releaseBranch} already exists.`);
  }
  requireMissingRemoteRef(remote, `refs/tags/${tag}`, tag);
  requireMissingRemoteRef(remote, `refs/heads/${releaseBranch}`, releaseBranch);

  console.log(`[release] Creating ${releaseBranch} for ${tag}...`);
  git(["switch", "-c", releaseBranch, `refs/remotes/${remote}/${mainBranch}`], { capture: false });
  execute(process.execPath, ["scripts/version-bump.mjs", bump], { capture: false });
  execute(process.execPath, ["scripts/verify-release-version.mjs", tag], { capture: false });
  console.log("[release] Running local release checks...");
  npmRun("release:check");

  git(["add", "--", ...versionFiles], { capture: false });
  git(["commit", "-m", `chore(release): ${tag}`], { capture: false });
  git(["push", "--set-upstream", remote, `HEAD:refs/heads/${releaseBranch}`], { capture: false });

  const prUrl = output("gh", [
    "pr",
    "create",
    "--base",
    mainBranch,
    "--head",
    releaseBranch,
    "--title",
    `chore(release): ${tag}`,
    "--body",
    `## Release\n\nBump Orcestr Real Translate to ${nextVersion}.\n\n## Checks\n\n- npm run release:check`,
  ]);
  console.log(`[release] Merging ${prUrl} with squash...`);
  execute("gh", ["pr", "merge", prUrl, "--squash"], { capture: false });

  const pullRequest = JSON.parse(output("gh", ["pr", "view", prUrl, "--json", "state,mergeCommit"]));
  if (pullRequest.state !== "MERGED" || !pullRequest.mergeCommit?.oid) {
    throw new Error(`Release PR was not merged: ${prUrl}`);
  }

  git(["switch", mainBranch], { capture: false });
  git(["fetch", remote, `refs/heads/${mainBranch}:refs/remotes/${remote}/${mainBranch}`], { capture: false });
  git(["merge", "--ff-only", `refs/remotes/${remote}/${mainBranch}`], { capture: false });
  const mergedHead = git(["rev-parse", "HEAD"]);
  if (mergedHead !== pullRequest.mergeCommit.oid) {
    throw new Error(`Release aborted: ${mainBranch} does not point to the merged release commit.`);
  }
  execute(process.execPath, ["scripts/verify-release-version.mjs", tag], { capture: false });

  git(["tag", "-a", tag, "-m", `Orcestr Real Translate ${tag}`], { capture: false });
  console.log(`[release] Pushing ${tag}; this starts the only CI/CD workflow.`);
  git(["push", remote, `refs/tags/${tag}:refs/tags/${tag}`], { capture: false });

  if (status("git", ["push", remote, "--delete", releaseBranch]) !== 0) {
    console.warn(`[release] Remote branch ${releaseBranch} could not be deleted automatically.`);
  }
  if (status("git", ["branch", "-d", releaseBranch]) !== 0) {
    console.warn(`[release] Local branch ${releaseBranch} could not be deleted automatically.`);
  }
  console.log(`[release] ${tag} is published. Follow the tag-triggered CI/CD run in GitHub Actions.`);
}

const invokedPath = process.argv[1] ? pathToFileURL(resolve(process.argv[1])).href : "";
if (invokedPath === import.meta.url) {
  try {
    runRelease(process.argv[2]);
  } catch (error) {
    console.error(`[release] ${error instanceof Error ? error.message : String(error)}`);
    process.exitCode = 1;
  }
}

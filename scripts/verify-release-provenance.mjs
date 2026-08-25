import { execFileSync } from "node:child_process";

const [tagRef, mainRef, expectedSha] = process.argv.slice(2);

if (!/^refs\/tags\/v\d+\.\d+\.\d+$/.test(tagRef ?? "")) {
  throw new Error("Release ref must be an exact stable tag ref such as refs/tags/v1.2.3");
}
if (mainRef !== "refs/remotes/origin/main") {
  throw new Error("Release ancestry must be checked against refs/remotes/origin/main");
}
if (!/^[0-9a-fA-F]{40}$/.test(expectedSha ?? "")) {
  throw new Error("Expected release SHA must be a full 40-character commit SHA");
}

function git(args) {
  return execFileSync("git", args, {
    encoding: "utf8",
    stdio: ["ignore", "pipe", "pipe"],
  }).trim();
}

const tagCommit = git(["rev-parse", "--verify", `${tagRef}^{commit}`]);
const eventCommit = git(["rev-parse", "--verify", `${expectedSha}^{commit}`]);
git(["rev-parse", "--verify", `${mainRef}^{commit}`]);

if (tagCommit !== eventCommit) {
  throw new Error(`Release tag resolves to ${tagCommit}, not event commit ${eventCommit}`);
}

try {
  execFileSync("git", ["merge-base", "--is-ancestor", tagCommit, mainRef], {
    stdio: "ignore",
  });
} catch {
  throw new Error(`Release commit ${tagCommit} is not contained in origin/main`);
}

process.stdout.write(`Release commit ${tagCommit} is contained in origin/main.\n`);

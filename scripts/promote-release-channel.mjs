import { spawnSync } from "node:child_process";
import { mkdtempSync, readFileSync, rmSync } from "node:fs";
import { readFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";

const [file, key] = process.argv.slice(2);
const endpoint = process.env.S3_ENDPOINT_URL;
const bucket = process.env.S3_BUCKET;

if (!file || !key || !endpoint || !bucket) {
  throw new Error(
    "Usage: promote-release-channel.mjs <manifest-file> <object-key> with S3_ENDPOINT_URL and S3_BUCKET",
  );
}
if (key.startsWith("/") || key.endsWith("/") || key.includes("..")) {
  throw new Error(`Unsafe S3 object key: ${key}`);
}

const candidateBytes = await readFile(file);
const candidate = parseManifest(candidateBytes, "Candidate release manifest");

function parseManifest(bytes, description) {
  let value;
  try {
    value = JSON.parse(bytes.toString("utf8"));
  } catch (error) {
    throw new Error(`${description} is not valid JSON`, { cause: error });
  }
  if (typeof value?.version !== "string" || !/^\d+\.\d+\.\d+$/.test(value.version)) {
    throw new Error(`${description} has no stable semantic version`);
  }
  return value;
}

function compareVersions(left, right) {
  const leftParts = left.split(".").map(Number);
  const rightParts = right.split(".").map(Number);
  for (let index = 0; index < 3; index += 1) {
    if (leftParts[index] !== rightParts[index]) {
      return leftParts[index] - rightParts[index];
    }
  }
  return 0;
}

function aws(args, { allowFailure = false } = {}) {
  const result = spawnSync("aws", args, { encoding: "utf8" });
  if (result.status !== 0 && !allowFailure) {
    throw new Error(`aws ${args[0]} failed: ${result.stderr || result.stdout}`);
  }
  return result;
}

function readCurrent() {
  const head = aws(
    [
      "s3api",
      "head-object",
      "--endpoint-url",
      endpoint,
      "--bucket",
      bucket,
      "--key",
      key,
      "--no-cli-pager",
    ],
    { allowFailure: true },
  );
  if (head.status !== 0) return null;

  const metadata = JSON.parse(head.stdout || "{}");
  const etag = String(metadata.ETag ?? "").replace(/^W\//, "").replace(/^\"|\"$/g, "");
  if (!etag) throw new Error(`Existing channel ${key} has no ETag`);

  const temporaryDirectory = mkdtempSync(join(tmpdir(), "orcestr-release-channel-"));
  const temporaryFile = join(temporaryDirectory, "manifest.json");
  try {
    aws([
      "s3",
      "cp",
      `s3://${bucket}/${key}`,
      temporaryFile,
      "--endpoint-url",
      endpoint,
      "--only-show-errors",
      "--no-progress",
      "--no-cli-pager",
    ]);
    return { bytes: readFileSync(temporaryFile), etag };
  } finally {
    rmSync(temporaryDirectory, { recursive: true, force: true });
  }
}

for (let attempt = 1; attempt <= 8; attempt += 1) {
  const current = readCurrent();
  if (current) {
    const currentManifest = parseManifest(current.bytes, `Existing channel ${key}`);
    const comparison = compareVersions(currentManifest.version, candidate.version);
    if (comparison > 0) {
      process.stdout.write(
        `Channel ${key} already points to newer ${currentManifest.version}; leaving it unchanged.\n`,
      );
      process.exit(0);
    }
    if (comparison === 0) {
      if (!current.bytes.equals(candidateBytes)) {
        throw new Error(`Refusing to mutate published ${key} version ${candidate.version}`);
      }
      process.stdout.write(`Channel ${key} already points to ${candidate.version}.\n`);
      process.exit(0);
    }
  }

  const condition = current ? ["--if-match", current.etag] : ["--if-none-match", "*"];
  const upload = aws(
    [
      "s3api",
      "put-object",
      "--endpoint-url",
      endpoint,
      "--bucket",
      bucket,
      "--key",
      key,
      "--body",
      file,
      ...condition,
      "--acl",
      "public-read",
      "--content-type",
      "application/json",
      "--cache-control",
      "no-cache, no-store, must-revalidate",
      "--no-cli-pager",
    ],
    { allowFailure: true },
  );
  if (upload.status === 0) {
    process.stdout.write(`Promoted ${key} to ${candidate.version}.\n`);
    process.exit(0);
  }
  if (attempt === 8) {
    throw new Error(`Could not atomically promote ${key}: ${upload.stderr || upload.stdout}`);
  }
}

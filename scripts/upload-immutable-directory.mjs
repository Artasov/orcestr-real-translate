import { createHash } from "node:crypto";
import { readFile, readdir, stat } from "node:fs/promises";
import { basename, join } from "node:path";
import { spawnSync } from "node:child_process";

const [directory, prefix] = process.argv.slice(2);
const endpoint = process.env.S3_ENDPOINT_URL;
const bucket = process.env.S3_BUCKET;
if (!directory || !prefix || !endpoint || !bucket) {
  throw new Error(
    "Usage: upload-immutable-directory.mjs <directory> <prefix> with S3_ENDPOINT_URL and S3_BUCKET",
  );
}
if (prefix.startsWith("/") || prefix.endsWith("/") || prefix.includes("..")) {
  throw new Error(`Unsafe S3 prefix: ${prefix}`);
}

function aws(args, { allowNotFound = false } = {}) {
  const result = spawnSync("aws", args, { encoding: "utf8" });
  if (allowNotFound && result.status !== 0) return null;
  if (result.status !== 0) {
    throw new Error(`aws ${args[0]} failed: ${result.stderr || result.stdout}`);
  }
  return result.stdout;
}

const names = (await readdir(directory)).sort();
if (names.length === 0) throw new Error(`No files found in ${directory}`);
for (const name of names) {
  if (basename(name) !== name) throw new Error(`Unsafe release filename: ${name}`);
  const path = join(directory, name);
  if (!(await stat(path)).isFile()) continue;
  const contents = await readFile(path);
  const sha256 = createHash("sha256").update(contents).digest("hex");
  const key = `${prefix}/${name}`;
  const headText = aws(
    [
      "s3api",
      "head-object",
      "--endpoint-url",
      endpoint,
      "--bucket",
      bucket,
      "--key",
      key,
    ],
    { allowNotFound: true },
  );
  if (headText !== null) {
    const head = JSON.parse(headText);
    if (
      Number(head.ContentLength) !== contents.length ||
      String(head.Metadata?.sha256 ?? "").toLowerCase() !== sha256
    ) {
      throw new Error(`Immutable S3 object differs from local release file: ${key}`);
    }
    continue;
  }

  const contentType = name.endsWith(".json")
    ? "application/json"
    : name.endsWith(".sig")
      ? "text/plain"
      : "application/octet-stream";
  aws([
    "s3api",
    "put-object",
    "--endpoint-url",
    endpoint,
    "--bucket",
    bucket,
    "--key",
    key,
    "--body",
    path,
    "--if-none-match",
    "*",
    "--acl",
    "public-read",
    "--metadata",
    `sha256=${sha256}`,
    "--content-type",
    contentType,
    "--cache-control",
    "public,max-age=31536000,immutable",
  ]);
  const uploaded = JSON.parse(
    aws([
      "s3api",
      "head-object",
      "--endpoint-url",
      endpoint,
      "--bucket",
      bucket,
      "--key",
      key,
    ]),
  );
  if (
    Number(uploaded.ContentLength) !== contents.length ||
    String(uploaded.Metadata?.sha256 ?? "").toLowerCase() !== sha256
  ) {
    throw new Error(`S3 verification failed after upload: ${key}`);
  }
}

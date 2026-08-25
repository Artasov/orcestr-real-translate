import { createHash } from "node:crypto";
import { readFile, stat } from "node:fs/promises";

const [command, file, headObjectFile] = process.argv.slice(2);
if (!/^(fingerprint|verify)$/.test(command ?? "") || !file) {
  throw new Error(
    "Usage: verify-immutable-object.mjs <fingerprint|verify> <local-file> [head-object.json]",
  );
}

const contents = await readFile(file);
const { size } = await stat(file);
const sha256 = createHash("sha256").update(contents).digest("hex");

if (command === "fingerprint") {
  process.stdout.write(`${sha256} ${size}\n`);
  process.exit(0);
}

if (!headObjectFile) {
  throw new Error("verify requires the JSON output from aws s3api head-object");
}

const head = JSON.parse(await readFile(headObjectFile, "utf8"));
const remoteSha256 = head.Metadata?.sha256;
const remoteSize = head.ContentLength;
if (remoteSha256 !== sha256 || remoteSize !== size) {
  throw new Error(
    `Immutable object mismatch: local sha256=${sha256} size=${size}, remote sha256=${String(remoteSha256)} size=${String(remoteSize)}`,
  );
}

process.stdout.write(`Immutable object matches sha256=${sha256} size=${size}.\n`);

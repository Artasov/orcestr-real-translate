import assert from "node:assert/strict";
import { execFile } from "node:child_process";
import { createHash } from "node:crypto";
import { mkdtemp, mkdir, readFile, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import { promisify } from "node:util";
import test from "node:test";

const run = promisify(execFile);
const projectRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");

test("release shortcuts calculate stable versions and invoke the PR/tag workflow", async () => {
  const releaseModule = await import(
    pathToFileURL(join(projectRoot, "scripts", "release.mjs")).href
  );
  assert.equal(releaseModule.nextStableVersion("1.2.3", "patch"), "1.2.4");
  assert.equal(releaseModule.nextStableVersion("1.2.3", "minor"), "1.3.0");
  assert.equal(releaseModule.nextStableVersion("1.2.3", "major"), "2.0.0");
  assert.throws(() => releaseModule.nextStableVersion("1.2.3-beta.1", "patch"));

  const packageJson = JSON.parse(await readFile(join(projectRoot, "package.json"), "utf8"));
  for (const bump of ["patch", "minor", "major"]) {
    assert.equal(packageJson.scripts[`release:${bump}`], `node scripts/release.mjs ${bump}`);
    const runConfiguration = await readFile(
      join(projectRoot, ".run", `${bump}.run.xml`),
      "utf8",
    );
    assert.match(runConfiguration, new RegExp(`<script value="release:${bump}"`));
  }
});

test("repository notifier follows the shared Telegram integration contract", async () => {
  const workflow = await readFile(
    join(projectRoot, ".github", "workflows", "orcestr-repo-notifier.yml"),
    "utf8",
  );
  assert.match(workflow, /push:\s*\r?\n\s+branches:\s*\r?\n\s+- main/);
  assert.match(workflow, /workflow_dispatch:/);
  assert.match(workflow, /Artasov\/orcestr-repo-notifier@v1/);
  for (const secret of [
    "OPENAI_API_KEY",
    "TELEGRAM_BOT_TOKEN",
  ]) {
    assert.match(workflow, new RegExp(`secrets\\.${secret}`));
  }
  assert.match(workflow, /telegram-chat-id: '@orcestrdev'/);
  assert.match(workflow, /telegram-message-thread-id: '2'/);
  assert.doesNotMatch(workflow, /secrets\.TELEGRAM_(?:CHAT_ID|MESSAGE_THREAD_ID)/);
  assert.match(workflow, /First line: DEV UPDATE <b>Orcestr Real Translate<\/b>:/);
  assert.doesNotMatch(workflow, /pull_request:/);
});

test("bundle collector enforces every matrix updater family and signature", async () => {
  const definitions = {
    windows: {
      runnerArch: "X64",
      files: [
        "Orcestr_1.2.3_x64-setup.exe",
        "Orcestr_1.2.3_x64-setup.exe.sig",
        "Orcestr_1.2.3_x64_en-US.msi",
        "Orcestr_1.2.3_x64_en-US.msi.sig",
      ],
      targets: ["windows-x86_64-nsis", "windows-x86_64-msi"],
    },
    darwin: {
      runnerArch: "ARM64",
      releaseArchitecture: "universal",
      files: ["Orcestr_1.2.3.dmg", "Orcestr.app.tar.gz", "Orcestr.app.tar.gz.sig"],
      targets: ["darwin-aarch64-app", "darwin-x86_64-app"],
    },
    linux: {
      runnerArch: "X64",
      files: [
        "Orcestr_1.2.3_amd64.AppImage",
        "Orcestr_1.2.3_amd64.AppImage.sig",
        "Orcestr_1.2.3_amd64.deb",
        "Orcestr_1.2.3_amd64.deb.sig",
      ],
      targets: ["linux-x86_64-appimage", "linux-x86_64-deb"],
    },
  };

  for (const [platform, definition] of Object.entries(definitions)) {
    const directory = await mkdtemp(join(tmpdir(), `orcestr-collect-${platform}-`));
    const bundleDirectory = join(directory, "bundle", "nested");
    const outputDirectory = join(directory, "output");
    await mkdir(bundleDirectory, { recursive: true });
    for (const name of definition.files) {
      await writeFile(
        join(bundleDirectory, name),
        name.endsWith(".sig") ? `signature-${name}` : "bundle",
        "utf8",
      );
    }

    await run(
      process.execPath,
      [
        join(projectRoot, "scripts", "collect-bundles.mjs"),
        join(directory, "bundle"),
        outputDirectory,
        platform,
      ],
      {
        cwd: projectRoot,
        env: {
          ...process.env,
          RUNNER_ARCH: definition.runnerArch,
          RELEASE_ARCHITECTURE: definition.releaseArchitecture ?? "native",
        },
      },
    );

    const metadata = JSON.parse(
      await readFile(join(outputDirectory, `${platform}-metadata.json`), "utf8"),
    );
    assert.deepEqual(
      metadata.updaters.map(({ target }) => target),
      definition.targets,
    );
    assert.equal(metadata.files.length, definition.files.length);
    assert.ok(metadata.files.every((name) => name.startsWith(`${platform}-`)));
    if (platform === "darwin") assert.equal(metadata.architecture, "universal");
  }
});

test("aggregate manifest is a valid bundle-specific Tauri updater manifest", async () => {
  const directory = await mkdtemp(join(tmpdir(), "orcestr-release-"));
  const definitions = {
    windows: ["windows-x86_64-nsis", "windows-x86_64-msi"],
    darwin: ["darwin-x86_64-app"],
    linux: ["linux-x86_64-appimage", "linux-x86_64-deb"],
  };

  for (const [platform, targets] of Object.entries(definitions)) {
    const files = targets.map((target) => `${target}.bundle`);
    await Promise.all(files.map((name) => writeFile(join(directory, name), "artifact", "utf8")));
    await writeFile(
      join(directory, `${platform}-metadata.json`),
      JSON.stringify({
        platform,
        files,
        updaters: targets.map((target, index) => ({
          target,
          updaterFile: files[index],
          signature: `signature-${target}`,
        })),
      }),
      "utf8",
    );
  }

  const versionManifest = join(directory, "manifest.json");
  const latestManifest = join(directory, "latest.json");
  const repeatedVersionManifest = join(directory, "manifest-repeated.json");
  const repeatedLatestManifest = join(directory, "latest-repeated.json");
  const sourceDateEpoch = "1712345678";
  await run(
    process.execPath,
    [
      join(projectRoot, "scripts", "build-release-manifest.mjs"),
      directory,
      "v1.2.3",
      "https://downloads.example.test/bucket/",
      versionManifest,
      latestManifest,
    ],
    { cwd: projectRoot, env: { ...process.env, SOURCE_DATE_EPOCH: sourceDateEpoch } },
  );
  await run(
    process.execPath,
    [
      join(projectRoot, "scripts", "build-release-manifest.mjs"),
      directory,
      "v1.2.3",
      "https://downloads.example.test/bucket/",
      repeatedVersionManifest,
      repeatedLatestManifest,
    ],
    { cwd: projectRoot, env: { ...process.env, SOURCE_DATE_EPOCH: sourceDateEpoch } },
  );

  const manifest = JSON.parse(await readFile(latestManifest, "utf8"));
  assert.equal(manifest.version, "1.2.3");
  assert.equal(manifest.pub_date, "2024-04-05T19:34:38.000Z");
  assert.equal(Object.keys(manifest.platforms).length, 5);
  assert.equal(
    manifest.platforms["windows-x86_64-nsis"].url,
    "https://downloads.example.test/bucket/orcestr-real-translate/v1.2.3/windows-x86_64-nsis.bundle",
  );
  assert.equal(
    manifest.platforms["linux-x86_64-deb"].signature,
    "signature-linux-x86_64-deb",
  );
  assert.deepEqual(
    JSON.parse(await readFile(versionManifest, "utf8")),
    JSON.parse(await readFile(latestManifest, "utf8")),
  );
  assert.equal(
    await readFile(repeatedVersionManifest, "utf8"),
    await readFile(versionManifest, "utf8"),
  );
  assert.equal(
    await readFile(repeatedLatestManifest, "utf8"),
    await readFile(latestManifest, "utf8"),
  );

  await assert.rejects(
    run(
      process.execPath,
      [
        join(projectRoot, "scripts", "build-release-manifest.mjs"),
        directory,
        "v1.2.3",
        "https://downloads.example.test/bucket/",
        join(directory, "invalid-time.json"),
      ],
      { cwd: projectRoot, env: { ...process.env, SOURCE_DATE_EPOCH: "not-a-time" } },
    ),
    /SOURCE_DATE_EPOCH/,
  );
});

test("download manifest exposes deterministic public installers for every desktop OS", async () => {
  const directory = await mkdtemp(join(tmpdir(), "orcestr-downloads-"));
  const definitions = {
    windows: {
      architecture: "x86_64",
      files: ["windows-Orcestr-setup.exe", "windows-Orcestr.msi"],
    },
    darwin: {
      architecture: "universal",
      files: ["darwin-Orcestr.dmg", "darwin-Orcestr.app.tar.gz"],
    },
    linux: {
      architecture: "x86_64",
      files: ["linux-Orcestr.AppImage", "linux-Orcestr.deb"],
    },
  };

  for (const [platform, definition] of Object.entries(definitions)) {
    for (const name of definition.files) {
      await writeFile(join(directory, name), `artifact-${name}`, "utf8");
    }
    await writeFile(
      join(directory, `${platform}-metadata.json`),
      JSON.stringify({
        platform,
        architecture: definition.architecture,
        files: definition.files,
        updaters: [],
      }),
      "utf8",
    );
  }

  const versionManifest = join(directory, "downloads-version.json");
  const channelManifest = join(directory, "downloads.json");
  await run(
    process.execPath,
    [
      join(projectRoot, "scripts", "build-download-manifest.mjs"),
      directory,
      "v1.2.3",
      "https://downloads.example.test/bucket/",
      versionManifest,
      channelManifest,
    ],
    {
      cwd: projectRoot,
      env: { ...process.env, SOURCE_DATE_EPOCH: "1712345678" },
    },
  );

  const manifest = JSON.parse(await readFile(channelManifest, "utf8"));
  assert.equal(manifest.version, "1.2.3");
  assert.equal(manifest.published_at, "2024-04-05T19:34:38.000Z");
  assert.deepEqual(Object.keys(manifest.targets), [
    "macos-universal",
    "linux-x64",
    "windows-x64",
  ]);
  assert.equal(manifest.targets["macos-universal"].length, 1);
  assert.equal(manifest.targets["macos-universal"][0].name, "darwin-Orcestr.dmg");
  assert.equal(manifest.targets["linux-x64"].length, 2);
  assert.equal(
    manifest.targets["windows-x64"][0].url,
    "https://downloads.example.test/bucket/orcestr-real-translate/v1.2.3/windows-Orcestr-setup.exe",
  );
  assert.equal(
    manifest.targets["windows-x64"][0].sha256,
    createHash("sha256")
      .update("artifact-windows-Orcestr-setup.exe")
      .digest("hex"),
  );
  assert.deepEqual(
    JSON.parse(await readFile(versionManifest, "utf8")),
    manifest,
  );
});

test("GitHub release notes expose S3 links and no GitHub asset path", async () => {
  const directory = await mkdtemp(join(tmpdir(), "orcestr-release-notes-"));
  for (const [platform, file] of Object.entries({
    windows: "windows-Orcestr-setup.exe",
    darwin: "darwin-Orcestr.dmg",
    linux: "linux-Orcestr.AppImage",
  })) {
    await writeFile(
      join(directory, `${platform}-metadata.json`),
      JSON.stringify({ platform, architecture: "x86_64", files: [file], updaters: [] }),
      "utf8",
    );
  }
  const generated = join(directory, "generated.md");
  const output = join(directory, "release.md");
  await writeFile(generated, "Fixed realtime audio.", "utf8");
  await run(process.execPath, [
    join(projectRoot, "scripts", "build-github-release-notes.mjs"),
    directory,
    "v1.2.3",
    "https://downloads.example.test/root/",
    generated,
    output,
  ]);
  const notes = await readFile(output, "utf8");
  assert.match(notes, /https:\/\/downloads\.example\.test\/root\/orcestr-real-translate\/v1\.2\.3\/windows-Orcestr-setup\.exe/);
  assert.match(notes, /intentionally contains no uploaded assets/);
  assert.doesNotMatch(notes, /github\.com\/.*\/download/);
});

test("version bump synchronizes all five version-bearing files without git", async () => {
  const directory = await mkdtemp(join(tmpdir(), "orcestr-version-"));
  await mkdir(join(directory, "src-tauri"), { recursive: true });
  const originalPackage = JSON.parse(await readFile(join(projectRoot, "package.json"), "utf8"));
  const [major, minor, patch] = originalPackage.version.split(".").map(Number);
  const expected = `${major}.${minor}.${patch + 1}`;
  for (const path of [
    "package.json",
    "package-lock.json",
    "src-tauri/Cargo.toml",
    "src-tauri/Cargo.lock",
    "src-tauri/tauri.conf.json",
  ]) {
    await writeFile(
      join(directory, path),
      await readFile(join(projectRoot, path)),
    );
  }

  const script = join(projectRoot, "scripts", "version-bump.mjs");
  await run(process.execPath, [script, "patch"], { cwd: directory });
  await run(process.execPath, [script, "--check"], { cwd: directory });

  const packageJson = JSON.parse(await readFile(join(directory, "package.json"), "utf8"));
  const packageLock = JSON.parse(await readFile(join(directory, "package-lock.json"), "utf8"));
  const tauriJson = JSON.parse(
    await readFile(join(directory, "src-tauri", "tauri.conf.json"), "utf8"),
  );
  const cargoToml = await readFile(join(directory, "src-tauri", "Cargo.toml"), "utf8");
  const cargoLock = await readFile(join(directory, "src-tauri", "Cargo.lock"), "utf8");

  assert.equal(packageJson.version, expected);
  assert.equal(packageLock.version, expected);
  assert.equal(packageLock.packages[""].version, expected);
  assert.equal(tauriJson.version, expected);
  assert.equal(
    cargoToml.match(/\[package\][\s\S]*?^version = "([^"]+)"/m)?.[1],
    expected,
  );
  assert.equal(
    cargoLock.match(
      /\[\[package\]\]\r?\nname = "orcestr-real-translate"\r?\nversion = "([^"]+)"/,
    )?.[1],
    expected,
  );
});

test("release provenance accepts only a tagged commit contained in origin/main", async () => {
  const directory = await mkdtemp(join(tmpdir(), "orcestr-provenance-"));
  const git = (args) => run("git", args, { cwd: directory });
  await git(["init", "-b", "main"]);
  await git(["config", "user.name", "Orcestr CI Fixture"]);
  await git(["config", "user.email", "ci-fixture@example.test"]);

  await writeFile(join(directory, "fixture.txt"), "first\n", "utf8");
  await git(["add", "fixture.txt"]);
  await git(["commit", "-m", "first"]);
  const releaseSha = (await git(["rev-parse", "HEAD"])).stdout.trim();
  await git(["tag", "v1.2.3"]);

  await writeFile(join(directory, "fixture.txt"), "second\n", "utf8");
  await git(["commit", "-am", "second"]);
  await git(["update-ref", "refs/remotes/origin/main", "HEAD"]);

  const script = join(projectRoot, "scripts", "verify-release-provenance.mjs");
  await run(
    process.execPath,
    [script, "refs/tags/v1.2.3", "refs/remotes/origin/main", releaseSha],
    { cwd: directory },
  );

  await git(["checkout", "--detach", releaseSha]);
  await writeFile(join(directory, "side.txt"), "side\n", "utf8");
  await git(["add", "side.txt"]);
  await git(["commit", "-m", "side"]);
  const sideSha = (await git(["rev-parse", "HEAD"])).stdout.trim();
  await git(["tag", "v9.9.9"]);

  await assert.rejects(
    run(
      process.execPath,
      [script, "refs/tags/v9.9.9", "refs/remotes/origin/main", sideSha],
      { cwd: directory },
    ),
    /not contained in origin\/main/,
  );
});

test("immutable object verification requires matching SHA-256 metadata and size", async () => {
  const directory = await mkdtemp(join(tmpdir(), "orcestr-immutable-"));
  const artifact = join(directory, "artifact.bin");
  const headObject = join(directory, "head-object.json");
  await writeFile(artifact, "signed updater artifact", "utf8");

  const script = join(projectRoot, "scripts", "verify-immutable-object.mjs");
  const fingerprint = (
    await run(process.execPath, [script, "fingerprint", artifact], { cwd: projectRoot })
  ).stdout.trim();
  const [sha256, size] = fingerprint.split(" ");
  await writeFile(
    headObject,
    JSON.stringify({ ContentLength: Number(size), Metadata: { sha256 } }),
    "utf8",
  );
  await run(process.execPath, [script, "verify", artifact, headObject], {
    cwd: projectRoot,
  });

  await writeFile(
    headObject,
    JSON.stringify({ ContentLength: Number(size), Metadata: { sha256: "0".repeat(64) } }),
    "utf8",
  );
  await assert.rejects(
    run(process.execPath, [script, "verify", artifact, headObject], {
      cwd: projectRoot,
    }),
    /Immutable object mismatch/,
  );
});

test("release workflow gates secrets and pins the auth SDK to a repository SHA", async () => {
  const workflow = await readFile(
    join(projectRoot, ".github", "workflows", "ci-release.yml"),
    "utf8",
  );
  const readme = await readFile(join(projectRoot, "README.md"), "utf8");
  const immutableUploader = await readFile(
    join(projectRoot, "scripts", "upload-immutable-directory.mjs"),
    "utf8",
  );

  assert.match(workflow, /AUTH_SDK_SHA: [0-9a-f]{40}/);
  assert.match(workflow, /^on:\r?\n\s+push:\r?\n\s+tags: \["v\*"\]/m);
  assert.doesNotMatch(workflow, /^\s+pull_request:/m);
  assert.doesNotMatch(workflow, /^\s+branches:/m);
  assert.doesNotMatch(workflow, /^\s+workflow_dispatch:/m);
  assert.doesNotMatch(workflow, /vars\.AUTH_(?:CORE|SDK)_SHA/);
  assert.match(
    workflow,
    /ref: \$\{\{ needs\.dependency-preflight\.outputs\.auth-sdk-sha \}\}/,
  );
  assert.doesNotMatch(workflow, /auth-core-v\d/);
  assert.match(workflow, /fetch-depth: 0/);
  assert.match(workflow, /verify-release-provenance\.mjs/);
  assert.match(
    workflow,
    /needs: \[dependency-preflight, quality, native-check, release-preflight\]/,
  );
  assert.match(workflow, /upload-immutable-directory\.mjs/);
  assert.match(immutableUploader, /"--if-none-match"/);
  assert.match(immutableUploader, /"public-read"/);
  assert.match(workflow, /build-download-manifest\.mjs/);
  assert.match(workflow, /promote-release-channel\.mjs/);
  assert.match(workflow, /vars\.S3_PUBLIC_BASE_URL/);
  assert.match(workflow, /vars\.S3_ENDPOINT_URL/);
  assert.match(workflow, /vars\.S3_BUCKET/);
  assert.match(workflow, /macos-26/);
  assert.match(workflow, /Prepare Tauri frontend directory/);
  assert.match(workflow, /universal-apple-darwin/);
  assert.match(workflow, /libpulse-dev/);
  assert.match(workflow, /Create GitHub release without GitHub-hosted assets/);
  assert.doesNotMatch(workflow, /actions\/(?:upload|download)-artifact/);
  assert.match(workflow, /\.assets \| length/);
  assert.match(workflow, /--format=%ct/);
  assert.match(workflow, /SOURCE_DATE_EPOCH: \$\{\{ steps\.release-time\.outputs\.source-date-epoch \}\}/);
  assert.match(readme, /Windows x64/);
  assert.match(readme, /universal macOS \(Apple Silicon \+ Intel\)/);
  assert.match(readme, /Linux x64/);
});

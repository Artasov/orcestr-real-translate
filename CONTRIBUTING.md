# Contributing

[Русская версия](./CONTRIBUTING.ru.md)

Orcestr Real Translate is a security-sensitive native audio application. Keep changes focused,
reviewable, and explicit about renderer/native boundaries.

## Development

The sibling `orcestr-auth` repository is required for a locked local install:

```powershell
npm run deps:install
npm run dev
```

Use checks proportional to the change. Before opening a pull request, run the relevant subset:

```powershell
npm run version:check
npm run typecheck
npm test
npm run test:tooling
npm run build:renderer
npm run test:rust
```

Do not edit `dist/`, `src-tauri/target/`, generated Tauri output, release packages, or updater
manifests by hand.

## Change checklist

Describe in the pull request:

- what changed and which user flow it affects;
- microphone, loopback, resampling, denoising, playback, latency, or feedback implications;
- renderer/native IPC contract changes;
- authentication, OAuth, credential-store, deep-link, CSP, updater, or release implications;
- Windows, macOS, and Linux impact, including any platform-specific limitation;
- English and Russian documentation updates;
- the exact commands used for verification;
- screenshots or a short recording for visual changes.

Never commit API keys, access or refresh tokens, signing material, local `.env` files, captured
audio, real transcripts, personal data, or production credentials.

## Pull requests

Use a concise title. Keep unrelated work in separate pull requests. Public behavior, settings
schema changes, new native permissions, dependency additions, release changes, and security
tradeoffs must be called out explicitly.

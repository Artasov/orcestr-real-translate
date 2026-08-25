# Security Policy

[Русская версия](./SECURITY.ru.md)

## Supported versions

Security fixes are handled on the default branch while the application remains in beta.

## Reporting a vulnerability

Do not open a public issue for vulnerabilities. Report security problems privately to the
maintainer through GitHub.

Include the affected version or commit, platform, attack scenario, required privileges, possible
data exposure, and a minimal reproduction. Never send real API keys, tokens, captured audio,
transcripts, signing keys, personal data, or production credentials.

## Sensitive areas

Extra review is required for changes involving:

- password login, OAuth state/PKCE, token exchange, refresh rotation, logout, or deep links;
- OS credential storage, API-key handling, IPC payloads, CSP, external navigation, or logging;
- microphone permissions, WASAPI loopback, device identifiers, audio routing, or feedback checks;
- Realtime WebSockets, provider errors, transcript bounds, audio bounds, or cancellation;
- Tauri capabilities, updater signatures, immutable artifacts, GitHub Actions, or S3 publishing;
- third-party native dependencies and their bundled models or licenses.

The renderer must never receive stored secrets or raw authentication tokens. Captured audio must
not be persisted unless a future feature introduces an explicit, reviewed user-controlled flow.

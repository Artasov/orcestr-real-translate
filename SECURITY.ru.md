# Политика безопасности

[English version](./SECURITY.md)

## Поддерживаемые версии

Пока приложение находится в beta, security fixes выпускаются из default branch.

## Сообщение об уязвимости

Не создавайте публичный issue для уязвимостей. Передайте информацию maintainer приватно через
GitHub.

Укажите версию или commit, платформу, сценарий атаки, необходимые права, возможную утечку данных и
минимальное воспроизведение. Не отправляйте реальные API keys, tokens, записи аудио, транскрипты,
signing keys, персональные данные и production credentials.

## Критичные зоны

Особого review требуют изменения, затрагивающие:

- password login, OAuth state/PKCE, token exchange, refresh rotation, logout и deep links;
- OS credential storage, обработку API key, IPC payloads, CSP, external navigation и logging;
- microphone permissions, WASAPI loopback, device identifiers, audio routing и feedback checks;
- Realtime WebSockets, provider errors, ограничения transcript/audio и cancellation;
- Tauri capabilities, updater signatures, immutable artifacts, GitHub Actions и S3 publishing;
- сторонние native dependencies, встроенные модели и их лицензии.

Renderer не должен получать сохранённые secrets или raw authentication tokens. Захваченный звук
не должен сохраняться без отдельного, явно управляемого пользователем и прошедшего review flow.

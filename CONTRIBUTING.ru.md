# Участие в разработке

[English version](./CONTRIBUTING.md)

Orcestr Real Translate — security-sensitive нативное аудиоприложение. Изменения должны быть
сфокусированными, удобными для review и сохранять явную границу между renderer и native code.

## Разработка

Для locked local install требуется соседний репозиторий `orcestr-auth`:

```powershell
npm run deps:install
npm run dev
```

Запускайте проверки, соответствующие изменению. Перед pull request используйте нужный набор:

```powershell
npm run version:check
npm run typecheck
npm test
npm run test:tooling
npm run build:renderer
npm run test:rust
```

Не редактируйте вручную `dist/`, `src-tauri/target/`, generated Tauri output, release packages и
updater manifests.

## Checklist изменения

В pull request укажите:

- что изменено и какой user flow затронут;
- влияние на microphone, loopback, resampling, denoising, playback, latency и feedback;
- изменения renderer/native IPC contracts;
- последствия для authentication, OAuth, credential store, deep links, CSP, updater и releases;
- влияние на Windows, macOS и Linux, включая platform-specific ограничения;
- обновления английской и русской документации;
- точные команды проверки;
- screenshots или короткую запись для visual changes.

Никогда не коммитьте API keys, access/refresh tokens, signing material, локальные `.env`, записи
звука, реальные транскрипты, персональные данные и production credentials.

## Pull requests

Используйте короткий понятный title. Независимые изменения разделяйте. Явно отмечайте изменения
public behavior, settings schema, native permissions, dependencies, release process и security
tradeoffs.

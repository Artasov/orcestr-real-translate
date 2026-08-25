<p align="right">
  <a href="./README.md">English</a> · <strong>Русский</strong>
</p>

<p align="center">
  <a href="https://orcestr.com">
    <img src="./assets/orcestr-real-translate-banner.png" alt="Баннер Orcestr Real Translate" width="100%" />
  </a>
</p>

# Orcestr Real Translate

[![CI / Release](https://github.com/Artasov/orcestr-real-translate/actions/workflows/ci-release.yml/badge.svg)](https://github.com/Artasov/orcestr-real-translate/actions/workflows/ci-release.yml)
[![Tauri 2](https://img.shields.io/badge/Tauri-2-24C8DB?logo=tauri&logoColor=white)](https://v2.tauri.app/)
[![License: MPL 2.0](https://img.shields.io/badge/License-MPL_2.0-brightgreen.svg)](./LICENSE)

Нативное приложение для двусторонней транскрибации и перевода речи в живых разговорах.

Orcestr Real Translate независимо захватывает микрофон, системный звук или оба канала сразу.
Каждое направление может показывать live-транскрипт, переводить речь на выбранный язык и по желанию
воспроизводить переведённый голос через собственный аудиовыход. История текста существует только
до закрытия процесса приложения.

Основной сайт: [orcestr.com](https://orcestr.com)

## Статус

| Пункт | Значение |
| --- | --- |
| Продукт | Orcestr Real Translate |
| Версия | `0.1.0` |
| Статус | Beta |
| Desktop runtime | Tauri 2 / Rust 2021 |
| Интерфейс | React 19 / TypeScript / `@orcestr/ui` |
| Платформы в CI | Windows x64, universal macOS (Apple Silicon + Intel), Linux x64 |

Приложение и его Realtime-контракты активно развиваются. До первого стабильного релиза текущую
схему настроек и release channel не следует считать стабильными.

## Возможности

| Зона | Поведение |
| --- | --- |
| Микрофон | Захват выбранного входа и улучшение речи до распознавания |
| Системный звук | Нативный WASAPI process loopback на Windows, ScreenCaptureKit на macOS и monitor default sink через PulseAudio/PipeWire на Linux |
| Транскрибация | Потоковое распознавание исходной речи с низкой задержкой |
| Перевод | Потоковый исходный текст, перевод и переведённый PCM-аудиопоток |
| Озвучка | Независимое включение и mute для каждого канала, в том числе во время сессии |
| Текст сессии | Копирование, очистка и автоматическое удаление при закрытии приложения |
| Маршрутизация | Отдельные устройства входа и выхода переведённой речи для каждого направления |
| Авторизация | Orcestr email/password и OAuth flow для public client |

## Аудиотракт

```text
микрофон ───────┐
                ├─ захват → RNNoise → adaptive gain → compressor → 24 kHz PCM16 → OpenAI Realtime
системный звук ─┘                                                                  │
                                                                                    ├─ исходный текст
                                                                                    ├─ перевод
                                                                                    └─ опциональная озвучка
```

Нативная обработка использует voice-aware подавление RNNoise, адаптивную оценку noise floor,
автоматическое усиление, компрессор, high-pass filter и limiter. Проверка маршрутов запрещает
выводить перевод в устройство, которое уже захватывается каналом системного звука. При выключенной
озвучке перевод остаётся текстовым и не требует аудиовыхода.

На macOS 13 и новее первая сессия системного аудио запрашивает разрешение **Screen & System Audio
Recording**; после его выдачи приложение нужно перезапустить. ScreenCaptureKit исключает собственную
озвучку приложения, поэтому перевод можно выводить в то же физическое устройство. На Linux захват
работает через PulseAudio-протокол, предоставляемый PulseAudio или PipeWire. Такой monitor не умеет
надёжно исключать один клиент, поэтому приложение блокирует озвучку обратно в захватываемый default
sink: нужно выбрать отдельный выход либо выключить озвучку системного канала.

## Приватность и безопасность

- OpenAI API key вводится в **Settings** и сохраняется в credential store операционной системы.
  Он не записывается в `.env` или browser storage и не возвращается в renderer.
- Access tokens остаются в памяти нативного процесса. Refresh credentials хранятся в OS keyring.
- Raw audio остаётся в Rust-процессе и отправляется напрямую в настроенный OpenAI Realtime endpoint.
  Renderer получает только ограниченные status, meter и transcript events.
- OAuth client является публичным: в нём нет client secret; используются PKCE, точная проверка state
  и зарегистрированный deep link `com.orcestr.realtranslate://oauth/callback`.
- Текст транскрипта хранится только в React memory текущей сессии и удаляется при закрытии приложения.

Перед сообщением об уязвимости или изменением авторизации, credentials, audio routing, updater либо
release boundary прочитайте [SECURITY.ru.md](./SECURITY.ru.md).

## Экосистема Orcestr

| Проект | Ответственность |
| --- | --- |
| [Orcestr Auth](https://github.com/Artasov/orcestr-auth) | Общие login forms, browser OAuth и desktop public-client contracts |
| [Orcestr UI](https://github.com/Artasov/orcestr-ui) | Компоненты, interaction patterns и visual tokens |
| [Orcestr Core](https://github.com/Artasov/orcestr-core) | Общие transport- и error-контракты |
| [Orcestr](https://github.com/Artasov/orcestr) | Product backend, OAuth registration и account services |

## Локальная разработка

Репозиторий авторизации должен лежать рядом: приложение сначала собирает его локальные packages,
а затем устанавливает desktop workspace.

```text
dev/
├── orcestr-auth/
└── orcestr-real-translate/
```

```powershell
npm run deps:install
npm run dev
```

После входа откройте **Settings**, сохраните OpenAI API key и выберите устройства захвата и
воспроизведения. Production endpoints встроены по умолчанию. В `.env.example` описан единственный
debug-only localhost override, принимаемый нативной проверкой конфигурации.

В `.run` находятся JetBrains-конфигурации для locked install, разработки, сборки, целевых тестов и
patch/minor/major version bump.

## Проверки

```powershell
npm run version:check
npm run typecheck
npm test
npm run test:tooling
npm run build:renderer
npm run test:rust
```

`npm run check` запускает полную последовательность проверок репозитория. Требования к изменениям
описаны в [CONTRIBUTING.ru.md](./CONTRIBUTING.ru.md).

## CI и релизы

Pull requests и `main` проверяют синхронизацию версий, TypeScript, renderer tests, release tooling,
production renderer build и Rust unit tests. Теги `vX.Y.Z` собирают Windows (`nsis`, `msi`), macOS
(`app`, `dmg`) и Linux (`AppImage`, `deb`) bundles.

Артефакты и подписи Tauri updater публикуются в неизменяемые versioned S3 prefixes. Канал
`latest.json` обновляется только после проверки всех платформенных артефактов. Release CI фиксирует
точную ревизию Orcestr Auth и требует, чтобы tagged commit входил в `origin/main`, до открытия
доступа к signing- и storage-secrets.
Платформенные jobs загружают файлы напрямую в S3: GitHub Actions artifacts и assets внутри GitHub
Release не используются. В GitHub Release публикуются только ссылки на скачивание из S3.

Перед первым release-тегом настройте **Repository variables**:

| Variable | Значение для текущего Orcestr Storage |
| --- | --- |
| `S3_REGION` | `ru-1` |
| `S3_ENDPOINT_URL` | `https://s3.twcstorage.ru` |
| `S3_BUCKET` | `324718a4-2cc5dd7a-917b-4e82-87c5-b9d5f8de16ba` |
| `S3_PUBLIC_BASE_URL` | `https://s3.twcstorage.ru/324718a4-2cc5dd7a-917b-4e82-87c5-b9d5f8de16ba/` |

Также настройте **Repository secrets**:

| Secret | Для чего нужен |
| --- | --- |
| `S3_ACCESS_KEY_ID` | Timeweb S3 access key с read/write-доступом к release prefix |
| `S3_SECRET_ACCESS_KEY` | Соответствующий Timeweb S3 secret key |
| `TAURI_SIGNING_PRIVATE_KEY` | Приватный Tauri updater key, соответствующий public key в `tauri.conf.json` |
| `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` | Пароль updater key; не задавайте для незашифрованного ключа |

Signing key не выдаётся Tauri, GitHub или S3: отдельную пару ключей Real Translate нужно один раз
сгенерировать локально. В PowerShell из корня проекта:

```powershell
New-Item -ItemType Directory -Force "$env:USERPROFILE\.tauri" | Out-Null
npx tauri signer generate -w "$env:USERPROFILE\.tauri\orcestr-real-translate.key"
```

Команда предложит придумать пароль и создаст private-файл
`orcestr-real-translate.key` и public-файл `orcestr-real-translate.key.pub`.

- содержимое `.key.pub` безопасно коммитится в `src-tauri/tauri.conf.json` как
  `plugins.updater.pubkey`;
- полное содержимое `.key` добавляется только в GitHub Actions secret
  `TAURI_SIGNING_PRIVATE_KEY`, но никогда не коммитится;
- придуманный при генерации пароль добавляется в `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`; если пароль
  оставлен пустым, этот secret не нужен.

До первого публичного релиза текущий public key, временно совпадающий с XEXAMAI, нужно заменить на
новый `.key.pub` Real Translate. После первого релиза пару ключей необходимо хранить в защищённом
backup: потеря private key лишит уже установленные версии возможности принять следующие обновления.
`GITHUB_TOKEN` GitHub Actions создаёт автоматически — добавлять его вручную не нужно. После успешного
релиза CI также обновляет `downloads.json`, из которого продуктовый лендинг получает прямые S3-ссылки.

## Правила репозитория

- [Участие в разработке](./CONTRIBUTING.ru.md)
- [Политика безопасности](./SECURITY.ru.md)
- [Code of Conduct](./CODE_OF_CONDUCT.md)
- [Использование бренда и trademarks](./TRADEMARKS.md)

## Maintainer

Публичные обновления ведёт [@Artasov](https://github.com/Artasov).

## Лицензия

Проект распространяется по [Mozilla Public License 2.0](./LICENSE). Использование, модификация и
коммерческое применение разрешены с file-level условиями MPL. Copyright и attribution notices
должны сохраняться, а изменения MPL-covered файлов остаются под MPL. Названия Orcestr, product
identity, логотипы и visual assets не передаются по MPL. См. [NOTICE](./NOTICE),
[TRADEMARKS.md](./TRADEMARKS.md) и [assets/README.md](./assets/README.md).

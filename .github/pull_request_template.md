## Summary

Describe the change briefly.

## Testing

- [ ] `npm run version:check`
- [ ] `npm run typecheck`
- [ ] `npm test`
- [ ] `npm run test:tooling`
- [ ] `npm run build:renderer`
- [ ] `npm run test:rust`
- [ ] Manual audio or visual check when relevant

## Desktop checklist

- [ ] Renderer/native IPC changes are documented
- [ ] Microphone, loopback, playback, latency, and feedback behavior were considered
- [ ] Windows, macOS, and Linux impact was considered
- [ ] English and Russian documentation was updated where needed

## Security

- [ ] This change does not expose keys, tokens, captured audio, transcripts, private URLs, or user data
- [ ] OAuth, credential-store, CSP, Tauri permissions, updater, and release implications were considered

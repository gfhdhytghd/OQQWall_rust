# TODO

## P0 - MVP gaps
- [x] Implement journal + snapshot + replay in `crates/infra` and wire into `crates/app/src/engine.rs` (append -> reduce -> publish, restore on boot).
- [x] Add a BlobStore (RAM cache + disk backup) and a MediaFetcher driver to handle `MediaFetchRequested` and update `MediaFetchSucceeded/Failed`.
- [x] Replace the render stub with a real PNG renderer per `docs/typesetting&render.md`.
- [x] Build an AuditPublisher driver that formats review messages (summary + preview link/PNG + attachments), sends to audit group, and emits `ReviewPublished` or retry events. (implemented in `crates/drivers/src/napcat.rs`)
- [ ] Expand config parsing to the full `docs/config.md` schema (groups, accounts, send windows, send_schedule, defaults, alias keys, env overrides), then feed `CoreConfig`. (`send_windows` and config-apply flow still missing)
- [x] Implement handling for `GroupFlushRequested` and send-queue flush logic; ensure schedule minutes from config are used.
- [x] Add missing failure events and retry logic (review publish fail, render fail backoff, media fetch fail backoff).
- [x] Extend NapCat command parsing to cover the full review/global command set in `docs/command.md`, with basic permission checks. (parser + admin gate)
- [x] Improve Qzone sender to use blob media/render outputs and handle missing drafts/retry attempts cleanly.

## P1 - Productization
- [x] Add admin web UI endpoints (queue/status, PNG preview, blob fetch) using axum. (`crates/app/src/web_api.rs`)
- [ ] Add tracing/metrics hooks (queue depth, send success rate, render latency, NapCat restarts).
- [ ] Implement managed NapCat mode (spawn, health checks, restart/backoff, multi-profile).
- [ ] Add config reload + `ConfigApplied` event, and avoid hardcoding group defaults.

## P2 - Hardening
- [x] Add core tests: reducer replay, tick idempotence, command parsing coverage. (see `crates/core/tests/*` and napcat parser tests)
- [ ] Add GC/retention for blobs/journal and size limits for render/download queues. (blob retention exists; journal GC/queue hard limits still pending)
- [ ] Add safety checks for render output (escape text, sanitize URLs, stable layout).

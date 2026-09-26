# AGENTS.md — OxygenRouter development guide

> **GitHub@OxygenAILab | OxygenAILab@StarsailsClover**

This document is for AI agents and human contributors working on OxygenRouter. It captures the engineering discipline and conventions.

## 1. Engineering principles

1. **Evidence before claims** — every behavior claim must be backed by code at `file:line`. No "should work" assertions without a verified test path.
2. **Modular planning** — large work (>=3 modules or >=500 lines) gets a todo list with status updates in real time.
3. **Edit safety** — no silent assumptions. If a function expects `id` but we have a `name`, surface the mismatch, don't paper over it.
4. **Audit integrity** — no fake reviews, no "tested locally" claims without a real test invocation and recorded output.
5. **Version discipline** — follow `v{Year}.{Major}-Alpha {AlphaVer.}` (BC convention). Current line: `v26.0-Alpha 1`.
6. **Watermark** — every code/document file should carry `GitHub@OxygenAILab | OxygenAILab@StarsailsClover` in the top module comment.

## 2. Project layout

```
OxygenRouter/
├── Cargo.toml                       # workspace root (5 member crates)
├── README.md
├── CHANGELOG.md
├── AGENTS.md
├── .gitignore
├── .devdocs/                        # design notes
├── .devlogs/                        # development session logs (gitignored)
├── docs/                            # long-form documentation
│   └── research/                    # verified upstream comparison studies
├── release/                         # packaged build output (gitignored)
├── releases/                        # built binaries (gitignored)
├── main.rs                          # legacy CLI entry (see crates/oxygenrouter-bin)
└── crates/
    ├── oxygenrouter-core/           # data models + SQLite
    ├── oxygenrouter-proxy/          # upstream client + scheduler + router
    ├── oxygenrouter-relay/          # provider adapters + protocol converters + SSE
    ├── oxygenrouter-webui/          # axum REST API + static webui
    │   └── web/                     # React 18 + Vite + Tailwind
    └── oxygenrouter-bin/            # CLI entry, Windows console subsystem
```

## 3. Tech stack & versions

| Layer        | Choice                              | Why                                       |
|--------------|-------------------------------------|-------------------------------------------|
| Backend      | Rust 1.75+ (stable MSVC)            | Memory safety + single-binary deployment  |
| HTTP server  | axum 0.7                            | Tokio-native, ergonomic                   |
| HTTP client  | reqwest 0.12 (rustls)               | Streaming support                         |
| Database     | SQLite via rusqlite (bundled)       | Zero external dependency                  |
| Frontend     | React 18 + Vite 5 + Tailwind 3      | Match OxygenClaw stack                    |
| Icons        | lucide-react                        | Match OxygenClaw                          |
| State        | @tanstack/react-query               | Server-state management                   |

## 4. Data model (SQLite)

16 tables, defined in `crates/oxygenrouter-core/src/db.rs`.

| Group | Table | Purpose |
|-------|-------|---------|
| Relay | `channels` | Upstream API providers (base_url, api_key, priority, weight, provider) |
| Relay | `api_keys` | Local client tokens |
| Relay | `model_maps` | Per-channel model rewriting (pattern → target) |
| Relay | `model_metadata` | Model registry (vendor, capabilities, pricing metadata) |
| Relay | `route_rules` | Route rules (priority / type / keyword based) |
| Relay | `request_logs` | Audit trail: method, path, model, channel, status, duration |
| Config | `settings` | KV for app config (listen host/port, local token, theme, language) |
| Config | `migrations` | Applied-migration ledger |
| Identity | `users` | Accounts + roles |
| Identity | `auth_sessions` | Session tokens |
| Billing | `ledger_entries` | Balance movements |
| Billing | `subscription_plans` | Plan catalog |
| Billing | `subscriptions` | Active/past subscriptions |
| Billing | `redemption_codes` | Redeemable codes |
| Billing | `redemption_uses` | Redemption audit (single-use enforcement) |
| Billing | `payment_orders` | Manual and gateway orders |

`crates/oxygenrouter-core/src/db.rs` holds the authoritative DDL; add a migration row to
`migrations` for every schema change.

## 5. Protocol surface

Implemented OpenAI-compatible endpoints (under `127.0.0.1:<port>/`):

Relay endpoints (13, registered in `crates/oxygenrouter-webui/src/proxy.rs`):

| Method | Path                       | Notes                                     |
|--------|----------------------------|-------------------------------------------|
| ANY    | /v1/chat/completions       | Streaming (SSE) + non-streaming           |
| ANY    | /v1/completions            | Legacy completions                        |
| ANY    | /v1/embeddings             | Pass-through                              |
| ANY    | /v1/responses              | Responses API                             |
| ANY    | /v1/messages               | Anthropic-format messages                 |
| ANY    | /v1/rerank                 | Rerank                                    |
| ANY    | /v1/audio/speech           | TTS                                       |
| ANY    | /v1/audio/transcriptions   | STT                                       |
| ANY    | /v1/audio/translations     | STT translation                           |
| ANY    | /v1/models                 | Model list (currently static; see P1)     |
| ANY    | /v1/images/generations     | Pass-through                              |
| ANY    | /v1/images/edits           | Pass-through                              |
| ANY    | /v1/images/variations      | Pass-through                              |

Admin / user API (60 routes, registered in `crates/oxygenrouter-webui/src/api.rs`), grouped:

| Group | Representative paths |
|-------|----------------------|
| Channels | `GET/POST /api/channels`, `GET/PUT/DELETE /api/channels/:id`, `POST .../test`, `POST .../models/fetch`, `GET/POST .../keys`, `PATCH/DELETE /api/channels/batch` |
| Keys | `GET/POST /api/keys`, `DELETE /api/keys/:id`, `GET /api/keys/usage`, `GET /api/keys/query` |
| Models | `GET/POST /api/models-metadata`, `PUT/DELETE /api/models-metadata/:id`, `GET .../missing`, `POST .../sync` |
| Routing | `GET/POST /api/model-maps`, `DELETE /api/model-maps/:id`, `GET/POST /api/rules`, `DELETE /api/rules/:id` |
| Logs | `GET /api/logs`, `GET /api/logs/stream`, `GET /api/log/stats` |
| System | `GET /api/status`, `GET /api/system/info`, `GET /api/dashboard`, `GET /api/analytics/flow`, `GET/PUT /api/settings`, `GET /api/options`, `PUT /api/options`, `POST /api/backup/create` |
| Auth | `POST /api/auth/{register,login,logout}`, `GET /api/auth/me` |
| Billing | `GET /api/wallet`, `GET /api/subscriptions/me`, `POST /api/subscriptions/subscribe`, `GET /api/plans`, `POST /api/redemption/redeem`, `POST /api/orders/manual` |
| Admin | `GET /api/admin/users`, `PUT /api/admin/users/:id`, `POST /api/admin/users/:id/balance-adjust`, `GET/PUT /api/admin/plans`, `GET/PUT /api/admin/redemption-codes`, `GET /api/admin/orders`, `POST /api/admin/orders/:id/complete`, `GET /api/admin/ledger` |

> **Adapter layer is wired (P1 complete for dispatch).** `oxygenrouter-proxy/src/dispatch.rs`
> resolves the channel's `ApiType`, builds the matching adaptor from `oxygenrouter-relay`,
> translates the request and response (including SSE), and extracts billing usage. See
> `docs/FACT.md` for the verified end-to-end evidence and the two items still open
> (incremental streaming, inbound-format response shaping).

## 6. Channel selection algorithm

```
pick_channel(exclude):
  enabled = SELECT * FROM channels WHERE enabled=1 AND id NOT IN (exclude)
  total_weight = SUM(weight) over enabled
  target = random() mod total_weight
  acc = 0
  for c in (sorted by priority DESC, weight DESC):
    acc += c.weight
    if acc > target: return c
```

Retry policy (default `max_retries=3`):
- 408, 425, 429, 500, 502, 503, 504, 401, 403 → switch channel
- network error → switch channel
- `context_length_exceeded` in body → try long-context fallback on same channel, then continue
- 400 (other) → return upstream error to client

## 7. Model routing heuristic (`model=auto`)

| Detected feature in request           | Resolved model              |
|---------------------------------------|-----------------------------|
| Path = /v1/images/*                   | `dall-e-3`                  |
| Path = /v1/audio/*                    | `tts-1`                     |
| Path = /v1/embeddings                 | `text-embedding-3-small`    |
| Message contains image_url            | `gpt-4o`                    |
| Total message chars > 16k             | `gpt-3.5-turbo-16k`         |
| tools/function field present          | `gpt-4o-mini`               |
| Default (plain chat)                  | `gpt-3.5-turbo`             |

Long-context fallback table (in `proxy::router`):
- `gpt-3.5-turbo` → `gpt-3.5-turbo-16k`
- `gpt-4` → `gpt-4-32k`
- `claude-3-haiku` → `claude-3-sonnet`
- `claude-3-sonnet` → `claude-3-opus`

## 8. Build & release

### Dev build

```bash
cargo build                       # debug
cargo build --release             # optimized (~2-5 MB binary)
```

### Frontend build (required for embedded WebUI)

```bash
cd crates/oxygenrouter-webui/web
npm install
npm run build
```

The `npm run build` outputs to `crates/oxygenrouter-webui/web/dist/`, which is then embedded via `include_dir!` at Rust compile time.

### Cross-platform targets

```bash
# Windows x86_64
cargo build --release --target x86_64-pc-windows-msvc

# Linux x86_64
rustup target add x86_64-unknown-linux-gnu
cargo build --release --target x86_64-unknown-linux-gnu

# macOS Apple Silicon
rustup target add aarch64-apple-darwin
cargo build --release --target aarch64-apple-darwin
```

## 9. Watermark rule

Every source file MUST start with a watermark in its module-level doc comment:

```rust
//! <file purpose>
//!
//! GitHub@OxygenAILab | OxygenAILab@StarsailsClover
```

For documentation files (`*.md`), put it on a comment line near the top.

## 10. Commit & PR conventions

- Use **Conventional Commits** (`feat:`, `fix:`, `chore:`, `docs:`, `refactor:`, `test:`)
- Subject ≤ 50 characters
- Body explains "why", not "what"
- One concern per commit

## 11. Testing checklist (before tagging an Alpha)

- [ ] `cargo build --release` succeeds
- [ ] `cargo test` passes (currently no tests; add when refactoring)
- [ ] `cd crates/oxygenrouter-webui/web && npm run build` succeeds
- [ ] Manual smoke: start binary, open WebUI, add a channel, run a chat, verify failover by toggling channels
- [ ] `git status` is clean (no stray build artifacts)
- [ ] `CHANGELOG.md` updated with this Alpha's notable changes

## 12. Known limitations (Alpha 1 → Alpha 2)

- ✅ Multi-user / quota / billing / subscriptions (SaaS stack) — landed in Alpha 1
- ✅ Provider adapter layer `oxygenrouter-relay` — 9 adaptors (OpenAI, Anthropic,
  Gemini, Bedrock+SigV4, Vertex, Ollama, Cohere, Azure, AdvancedCustom) with
  bidirectional OpenAI↔Claude / OpenAI↔Gemini conversion and SSE translation.
  See `docs/SUPERSET_ROADMAP.md` for the NewAPI-parity plan.
- ✅ Relay layer is wired into the live proxy path (`oxygenrouter-proxy/src/dispatch.rs`)
  and billing runs on it: pre-consume → settle → refund, with the wallet, key usage
  and `request_logs.tokens_used` all moving on real requests. See `docs/FACT.md`.
- Still open: relay streaming is buffered before adaptor translation (time-to-first-token).
- The **console** (`/api/*`) is gated by a deny-by-default access class — public,
  user, admin, root — applied as one middleware over the whole tree rather than
  per handler, so a new route fails closed. Root-only surfaces are those that
  expose or rewrite instance configuration (`options`, `settings`, `system/info`,
  `backup`, `plugin`, `system-task`). See `docs/FACT.md`.
- The **relay** credential check is permissive by design: `/v1/*` accepts any
  `Bearer xxx`, because channel-only deployments have no local key registry. This
  is not the console path, which does authenticate.
- No HTTPS for local server (rely on local trust)
- Model map uses simple glob patterns; no full regex
- Logs are in SQLite only; no streaming export

## 13. Billing invariants (do not break these)

The quota engine (`oxygenrouter-billing`) and its storage primitives
(`oxygenrouter-core/src/db.rs`) encode decisions that are easy to regress:

1. **Reserve atomically.** `try_reserve_key_quota` / `try_reserve_wallet` perform the guard and the
   mutation in one statement. Never split them into a read-then-write; concurrent requests would
   over-spend.
2. **The billing session owns balance movement.** `record_charge` / `append_ledger_entry` are
   **audit-only** writes. Moving money in both the session and the ledger double-charges.
3. **A settled session never refunds.** Otherwise success followed by a late failure is free.
4. **A failed request costs nothing.** Refund in full on the error path.
5. **`credit_tx` rejects negative balances; settlement does not.** Top-ups and admin credits must
   not go below zero, while a stream that overran its reservation must be able to record the debt.
6. **Zero `quota_micros` means unlimited** for a key, matching NewAPI. A per-call model price
   (`model_price`) counts as priced even with no token ratio.

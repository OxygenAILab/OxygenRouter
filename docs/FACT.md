# FACT.md — OxygenRouter project facts

> **GitHub@OxygenAILab | OxygenAILab@StarsailsClover**

This document records verified project facts. Update it whenever a fact changes.

## Verified facts (as of 2026-09-25)

### Project identity
- **Name**: OxygenRouter
- **Symbol**: O₂
- **Repository**: `https://github.com/OxygenAILab/OxygenRouter`
- **License**: MIT
- **Version**: `v26.0-Alpha 1`
- **Project root**: `C:\Users\Sails\Documents\Workspace\01-Active\Core-Systems\OxygenRouter`

### Tech stack (locked)
- **Backend language**: Rust 1.95 (MSVC) on Windows
- **HTTP server**: axum 0.7
- **HTTP client**: reqwest 0.12
- **Database**: SQLite (rusqlite 0.31, bundled)
- **Frontend framework**: React 18.2
- **Build tool**: Vite 5
- **CSS**: Tailwind CSS 3.3
- **Icons**: lucide-react 0.294
- **State management**: @tanstack/react-query 5

### Implemented relay endpoints (13, `crates/oxygenrouter-webui/src/proxy.rs`)
- `/v1/chat/completions` — streaming (SSE) + non-streaming
- `/v1/completions`
- `/v1/embeddings`
- `/v1/responses`
- `/v1/messages` (Anthropic-format)
- `/v1/rerank`
- `/v1/audio/speech`, `/v1/audio/transcriptions`, `/v1/audio/translations`
- `/v1/images/generations`, `/v1/images/edits`, `/v1/images/variations`
- `/v1/models` (currently a static list; dynamic-from-DB is pending)

### Provider adapters — now wired (2026-09-25)

`oxygenrouter-relay` supplies 9 provider adapters (OpenAI, Anthropic, Gemini,
Bedrock+SigV4, Vertex, Ollama, Cohere, Azure, AdvancedCustom) plus
OpenAI↔Claude / OpenAI↔Gemini converters and SSE state machines.

The wiring lives in `oxygenrouter-proxy/src/dispatch.rs`: for every attempt the
scheduler resolves the channel's `ApiType`, builds the matching adaptor,
translates the inbound body into that provider's wire format, and translates the
response (including SSE) back — extracting billing usage on the way. Pass-through
providers take the same path via `RelayFormat::Raw`, so there is one code path.

Per-request adaptor work completed in this change:

- `RelayInfo` now carries `api_key` / `credential_raw` / `request_path`, so
  adaptors can emit provider auth (previously `setup_headers` discarded `_info`
  and no adapter could ever send a credential).
- Real auth per provider: `Authorization: Bearer` (OpenAI, Cohere, Ollama,
  AdvancedCustom, Bedrock API-key mode), `x-api-key` (Anthropic),
  `x-goog-api-key` (Gemini), `api-key` + `?api-version` (Azure),
  `Authorization: Bearer` + project/location path (Vertex), SigV4 (Bedrock AKSK).
- `Adaptor::sign_request` hook added so body-signing providers (SigV4) can run
  after the body is serialized.
- URL construction per provider: OpenAI base+path with `/v1` de-duplication,
  Anthropic `/v1/messages`, Gemini/Vertex `:generateContent` & `:streamGenerateContent`,
  Azure `/openai/deployments/{model}/...`, Bedrock `/model/{id}/invoke[-with-response-stream]`.
- `GET /v1/models` is now built from the database (enabled channels' `model_list`
  plus the model registry) instead of a hard-coded list.

**Verified end-to-end** against the live NewAPI instance on `127.0.0.1:3000` used
as an upstream channel:

| Check | Result |
|---|---|
| Non-streaming chat via the OpenAI adaptor | `200`, real usage `prompt=18 completion=16` |
| Streaming chat (SSE) | `200 text/event-stream`, 25 `data:` frames |
| Channel configured as `provider=anthropic` (Claude path) | `200`, Claude→OpenAI response conversion, usage preserved |
| `GET /v1/models` | DB-derived: the channel's model plus `auto` |
| Request log | every attempt recorded with channel id, status, duration |

**Still open:** the relay client buffers streaming bodies before adaptor
translation, so time-to-first-token is not yet incremental; and inbound-format
selection is wired but response shaping for native `/v1/messages` clients still
returns OpenAI-shaped JSON. Both are tracked in the superset analysis (§5.2).

### Admin / user API
60 routes in `crates/oxygenrouter-webui/src/api.rs`. Groups: channels, keys,
models, routing, logs, system, auth, billing, admin.

### Implemented WebUI pages (18 routes, `web/src/App.tsx`)
Substantive: Overview, Model Analytics, Channels, API Keys, Routes, Request Logs,
Models, Model Registry, System Info, System Settings, Playground, Settings.

Placeholder stubs (4–20 lines each): Sign-in, Sign-up, Wallet, Plans,
Subscriptions, Admin Users, Admin Billing.

### Database
16 tables in `crates/oxygenrouter-core/src/db.rs`: `channels`, `api_keys`,
`model_maps`, `model_metadata`, `route_rules`, `request_logs`, `settings`,
`migrations`, `users`, `auth_sessions`, `ledger_entries`, `subscription_plans`,
`subscriptions`, `redemption_codes`, `redemption_uses`, `payment_orders`.

### Test & build baseline (2026-09-25)
- `cargo test --workspace` → **163 passed / 0 failed**, zero build warnings
  (`billing` 102 unit + 3 oracle-differential, `relay` 15 unit + 20 adaptor-contract,
  `webui` 8 billing-store + 5 usage-mapping, `core` 5 unit + 5 config-compat)
- `npm run build` (tsc + vite) → **passes** (2,032 modules)
- `cargo check --workspace` → clean

### Billing is live on the request path (verified end to end)

Pre-consume → settle → refund now runs on real requests. Verified against a live
upstream (the running NewAPI instance) on an isolated instance:

| Check | Evidence |
|---|---|
| Wallet debited | `100,000,000 → 99,999,969` µ$ for one chat request |
| Key usage moved | `api_keys.used_micros 0 → 31` |
| Tokens recorded | `request_logs.tokens_used = 30` (was always `NULL` before) |
| Charge arithmetic | `p=14, c=16` → `(14 + 16*3)/1e6 * 500000` = **31** ✓ |
| Ledger written | `consume -31 "glm-5.3-flash via openai (tiered_expr)"` |
| Ledger ⇄ wallet reconcile | `sum(ledger.amount_micros) == balance` exactly |
| Streaming billed | SSE `200 text/event-stream`, 26 frames, `tokens_used=38`, charge `-43` |
| **Failure refunds in full** | forced `502` from a dead upstream left the wallet **unchanged** (no ledger entry) |
| Per-call models | `dall-e-3` charges with zero tokens |

Three real gaps were found and fixed while wiring this, none of which unit tests
alone would have caught:

1. **`chat_completions` bypassed billing entirely.** Four handlers
   (`chat_completions`, `text_completions`, `embeddings`, `image_generations`)
   each re-implemented the dispatch logic instead of using the shared helper, so
   the billing added to that helper never ran for the most-used endpoint. They
   now delegate to one path (~240 duplicated lines removed).
2. **Streaming was charged zero.** `dispatch_openai` hard-coded `stream: false`
   when building the upstream request, so an SSE request was billed as an empty
   non-streaming one and the response lost its `text/event-stream` content type.
3. **`api_keys` had no owner.** Billing needs to know whose wallet pays; the
   table had no `user_id`, so no request could be attributed. Added as an
   idempotent migration, with `ApiKey::owned_by` for construction.

Also fixed: the `Footer` option default leaked another project's watermark
(`GitHub@NDBlockConnect | BlockConnect@StarsailsClover`); it now carries this
repository's mark.

### Billing engine (P2) — verified against live production traffic

`oxygenrouter-billing` implements the quota engine. Two pricing paths exist in
NewAPI and both are now supported:

1. **Tiered expressions** (the modern default, and what the live instance
   actually uses — `billing_mode: "tiered_expr"`). An operator writes a small
   expression in USD per million tokens:

   ```text
   tier("base", p * 3 + c * 12 + cr * 0.06)
   quota = round(expr_USD / 1_000_000 * QuotaPerUnit * group_ratio)
   ```

   `crates/oxygenrouter-billing/src/expr.rs` implements the language: token
   variables with **auto-exclusion**, `tier`, `fixed`, `param`, `header`, `u`,
   `has`, `min`/`max`/`abs`/`ceil`/`floor`, `hour`/`minute`/`weekday`/`month`/
   `day` with fixed-offset timezones, ternaries, and the full operator set.

2. **Classic ratios** (`chat_quota.rs`) — model/completion/cache/image ratios
   with the ±1 minimum-charge rule, for operators still on the legacy tables.

**Differential parity result.** `crates/oxygenrouter-billing/tests/fixtures/live_newapi_quota_oracle.json`
is a verbatim export of every `tiered_expr` request in the running instance:
**15,728 requests spanning 33 distinct expressions**. `live_oracle_diff.rs`
replays all of them through our engine.

| Check | Result |
|---|---|
| Requests priced identically to NewAPI | **15,728 / 15,728 (0 mismatches)** |
| Branch (`tier`) selection agreement | **all** |
| Distinct live expressions evaluated | **33 / 33** |
| Expressions covering the traffic | 2 expressions account for 13,590 requests |

This is the strongest available evidence for gate **G6**: not hand-picked
examples, but real production traffic including `cc1h` split cache tiers,
time-gated peak/off-peak rates, `len`-conditioned long-context tiers, and a
request-body-gated branch.

Also implemented: `quota_math` (int32 saturation, half-away-from-zero rounding,
the wider 2^53-1 wallet domain), `estimator` (per-vendor heuristic plus
optional cl100k BPE), and `session` (reserve → settle → refund, with an
idempotent refund and the invariant that **a settled session can never refund**;
a concurrent 20-thread test proves reservation cannot overspend).

### Fixed defects (2026-09-25)

1. **Config loss on partial `config.json`.** `AppSettings` had no per-field serde
   defaults, so any hand-written config that omitted a field failed to
   deserialize; startup then saved stock defaults over the operator's file,
   silently discarding settings such as a custom listen port. Found while
   bringing up an isolated end-to-end instance. Fixed by adding
   `#[serde(default = "...")]` to every field (regression test:
   `crates/oxygenrouter-core/tests/config_compat.rs`) and by making startup
   refuse to write back a config it could not parse.
2. **AWS region parsing rejected three-segment regions.** `is_aws_region` only
   accepted `area-N`, so `ap-southeast-2` failed and Bedrock fell back to
   `us-east-1`. Caught by adaptor contract tests.

### Failure modes that trigger channel switch
- HTTP 401, 403, 408, 425, 429
- HTTP 500, 502, 503, 504
- Network/timeout errors
- `context_length_exceeded` (triggers long-context model fallback first)

### WebUI design language (OxygenOrigin dark)
- Background `#0a0a0a`, surface `#141414`, on-surface `#fafafa`
- Font: Inter, JetBrains Mono for code
- Easing: `cubic-bezier(0.25, 0.46, 0.45, 0.94)` (Apple-style)
- Card radius: 12px, input/button radius: 8px, badge: 9999px

### File count snapshot
- Rust files: 45 (9,569 lines)
- TypeScript/TSX files: 41 (8,381 lines)
- CSS files: 1 (`web/src/styles/global.css`)
- Backend HTTP routes: 73 (60 admin/user + 13 relay)

### Upstream comparison (superset target)
- Superset baseline: **`QuantumNous/new-api`** (AGPL-3.0) @ `c2b7a9a` —
  ~471,100 LOC (217,437 Go + 253,686 frontend), 328 backend routes,
  65 canonical client routes, 40 admin settings sections, 40 provider adapters,
  38 DB tables, 7 locales × 6,778 keys.
- Full verified study: `docs/research/NEWAPI_SUPERSET_ANALYSIS.md`
- **License constraint:** NewAPI is AGPL-3.0; this project is MIT. NewAPI may be
  read as a behavioral specification only — its code must never be copied.

## Open questions
- _None recorded_

## Pending decisions
- _None recorded_

## References
<!-- Oxy g en AI Lab@ Star sails Clo ver -->
- Superset baseline: [NewAPI](https://github.com/QuantumNous/new-api) (AGPL-3.0 — behavioral spec only)
- Predecessor: [one-api](https://github.com/songquanpeng/one-api)
- WebUI style: [OxygenClaw](https://github.com/OxygenAILab/oxygen-claw) (`packages/webui/src/styles/global.css`)
- BC dev convention: `bc-developmentndebugging` skill (v26.0-alpha.7)

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

### Implemented relay endpoints

Grouped below; the authoritative list is `SUPPORTED_ENDPOINTS` in
`crates/oxygenrouter-webui/src/proxy.rs` (38 entries, and every one is driven
through the real router by `proxy::tests::every_supported_endpoint_is_registered_in_the_router`).
- `/v1/chat/completions` — streaming (SSE) + non-streaming
- `/v1/completions`
- `/v1/embeddings`, `/v1/engines/:model/embeddings`
- `/v1/responses`
- `/v1/responses/compact`, `/v1/alpha/search`
- `/v1/messages` (Anthropic-format), `/v1/messages/count_tokens`
- `/v1/rerank`
- `/v1/moderations`, `/v1/edits`
- `/v1beta/models/*path` — Gemini native inbound
- `/v1/files`, `/v1/files/:id`, `/v1/files/:id/content`
- `/v1/batches`, `/v1/batches/:id`, `/v1/batches/:id/cancel`
- `/v1/fine-tunes` (+ `:id`, `:id/cancel`, `:id/events`)
- `/v1/audio/speech`, `/v1/audio/transcriptions`, `/v1/audio/translations`
- `/v1/images/generations`, `/v1/images/edits`, `/v1/images/variations`
- `/v1/models` (three dialects), `/v1beta/models`, `/v1beta/openai/models`,
  `/v1/models/:model` (both list routes derived from the database)

### Client dialect is honoured on an OpenAI channel (verified 2026-09-26)

A client that speaks Anthropic (`/v1/messages`) or Gemini
(`/v1beta/models/<m>:generateContent`) gets its own dialect back — and, the part
that was broken, gets its **content** — even when the selected channel speaks
OpenAI. `OpenAiAdaptor::request_url` addresses such a request to
`{base}/v1/chat/completions` and `convert_request` translates the body into OpenAI
chat shape. The reference behaves the same way
(`relay/channel/openai/adaptor.go:180-184` hard-codes the chat route for a
Claude- or Gemini-format relay).

New converters, each with its own tests:

| Direction | File |
|---|---|
| Anthropic request → OpenAI request | `convert/claude_to_openai_request.rs` |
| OpenAI response → Anthropic response | `convert/openai_to_claude_response.rs` |
| OpenAI SSE → Anthropic SSE | `sse/openai.rs::to_claude_events` |
| Gemini request → OpenAI request | `convert/gemini_to_openai_request.rs` |
| OpenAI response → Gemini response | `same file` |
| OpenAI SSE → Gemini SSE | `same file` |

Evidence, every client/channel quadrant, live against the reference instance:

| Client | Channel | Observed |
|---|---|---|
| OpenAI | OpenAI | `chat.completion`, `content='PING'`, usage `10/2/12` |
| Claude | OpenAI | `content=[{"type":"text","text":"PING"}]`, usage `input=10 output=2` |
| Claude | Anthropic | pass-through, `content 'PING'` |
| OpenAI | Anthropic | translated, `content 'PING'` |
| Gemini | OpenAI | `parts:[{text:"PING"}]`, `finishReason STOP`, usage `prompt 10 / cand 2 / total 12` |

Streaming through an OpenAI channel. Claude client: `message_start`,
`content_block_start`, deltas, `content_block_stop`, `message_delta`,
`message_stop`, with real text and `tokens_used=2033` on the log row (it was `0`
while streaming was broken). Gemini client: Gemini-shaped `data:` frames, the
role-only opening delta correctly dropped, terminal frame carrying
`finishReason: STOP` and `usageMetadata` (`totalTokenCount 2025`).

**Why the Claude bug survived review:** the earlier check asserted *shape* only
(`type == "message"`, `content` is a list). An empty answer satisfies both. The
regression guards therefore assert on the request URL and on the extracted text:
`tests/openai_channel_serves_claude_clients.rs` and
`tests/openai_channel_serves_gemini_clients.rs`. Removing either the URL guard or
the request translation makes them fail (mutation-verified, both directions).

### Streaming asks the upstream to report usage

`claude_to_openai_request::convert` sets `stream_options.include_usage` on any
streamed request. Without it an OpenAI upstream omits the terminal usage chunk and
the request settles at zero — the same defect class as the hard-coded
`stream: false` fixed earlier.

### The passthrough path honours streaming (fixed 2026-09-26)

`relay_passthrough` hard-coded `stream: false`, so every pass-through endpoint
asked its upstream for a buffered reply and then returned it as
`application/json`. The visible symptom was a Gemini client posting to
`/v1beta/models/<m>:streamGenerateContent` receiving one JSON object instead of
SSE, which the Gemini client libraries cannot parse at all — they wait for a
stream that never arrives.

Detection is now three-valued (`passthrough_wants_stream`): a `stream: true`
body, an `Accept: text/event-stream` header, or a `:streamGenerateContent` method
name. The third exists because Gemini selects streaming through the URL alone and
says nothing in either the body or the headers.

`is_stream_request` also now treats an explicit `stream: false` as terminal
rather than falling through to `Accept`. An SDK that always sends
`Accept: text/event-stream` while asking for a buffered reply would otherwise be
marked streaming and its JSON body handed to the SSE parser.

Verified live: a Gemini client streaming through an OpenAI channel receives 18
Gemini-shaped frames ending in `finishReason: STOP` with `usageMetadata`
(`totalTokenCount 2025`), where before it received a single JSON object.

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
| `GET /v1/models` | DB-derived, three dialects (see below): channel models, registry entries, plus `auto` |
| Request log | every attempt recorded with channel id, status, duration |

**Still open:** the relay client buffers streaming bodies before adaptor
translation, so time-to-first-token is not yet incremental. (The other item that
was listed here — native `/v1/messages` clients receiving OpenAI-shaped JSON — is
fixed; see "Client dialect is honoured on an OpenAI channel".)

### Model listing speaks three dialects (2026-09-26)

`GET /v1/models` answered OpenAI shape to every client, so an Anthropic or Gemini
SDK could not parse it, and `GET /v1beta/models` — the Gemini client's
service-discovery route — 404'd outright. The reference resolves the dialect from
the credential headers (`router/relay-router.go:25-43`): `x-api-key` plus
`anthropic-version` means Anthropic, `x-goog-api-key` (or `?key=`) means Gemini.
We now do the same, in `crates/oxygenrouter-webui/src/model_list.rs`.

Field names are an exact match against the live reference, verified by comparing
the key sets rather than eyeballing the output:

| Dialect | Top level | Item |
|---|---|---|
| OpenAI | `data`, `object`, `success` | `created`, `id`, `object`, `owned_by`, `supported_endpoint_types` |
| Anthropic | `data`, `first_id`, `has_more`, `last_id` | `created_at`, `display_name`, `id`, `type` |
| Gemini | `models`, `nextPageToken` | `baseModelId`, `description`, `displayName`, `inputTokenLimit`, `maxTemperature`, `name`, `outputTokenLimit`, `supportedGenerationMethods`, `temperature`, `thinking`, `topK`, `topP`, `version` |

`created_at` is RFC 3339 (`2021-07-20T10:40:00Z`), not a unix integer — the field
type, not just its name, is what an Anthropic SDK validates.

Routes added: `GET /v1beta/models` (Gemini discovery) and
`GET /v1beta/openai/models` (the OpenAI-compatible alias the reference also
serves). Verified live: all three credential styles return their own dialect, and
both new routes answer the right shape.

**A route can exist and still be wrong.** `/v1beta/models` sits beside the
`/v1beta/models/*path` wildcard, which swallows the exact path too — so a missing
exact route answers `200` with the wildcard's body and an http-status check cannot
see it. The guard therefore asserts the *shape* through the real router:
`proxy::tests::the_model_listing_routes_answer_their_own_dialect` and
`…::the_model_list_dialect_follows_the_client_credentials`. Removing either route,
or bypassing the dialect detection, makes them fail (all three mutation-verified).

### Admin / user API
60 routes in `crates/oxygenrouter-webui/src/api.rs`. Groups: channels, keys,
models, routing, logs, system, auth, billing, admin.

### Implemented WebUI pages (18 routes, `web/src/App.tsx`)
Substantive: Overview, Model Analytics, Channels, API Keys, Routes, Request Logs,
Models, Model Registry, System Info, System Settings, Playground, Settings.

Placeholder stubs (4–20 lines each): Sign-in, Sign-up, Wallet, Plans,
Subscriptions, Admin Users, Admin Billing.

### Database
17 tables in `crates/oxygenrouter-core/src/db.rs`: `channels`, `api_keys`,
`model_maps`, `model_metadata`, `vendors`, `route_rules`, `request_logs`,
`settings`, `migrations`, `users`, `auth_sessions`, `ledger_entries`,
`subscription_plans`, `subscriptions`, `redemption_codes`, `redemption_uses`,
`payment_orders`.

### Vendors (2026-09-26)

`vendors` and its `/api/vendors` group (list / search / get / create / update /
delete, the paths the reference uses — `router/api-router.go:376-386`). It was
missing entirely; the reference keeps 44 rows.

`model_count` is derived by a correlated subquery over `model_metadata`, not
stored. The reference makes the same choice (`model/vendor_meta.go:15-25`, field
tag `gorm:"-"`), and the difference is observable: deleting a model with no other
write lowers the count, which a stored column could not do. A test asserts exactly
that (`crates/oxygenrouter-core/tests/vendors.rs`).

Verified live: create three, refuse a duplicate name with a readable message,
list in name order with counts, attach two models and watch OpenAI's count become
2, substring search, rename, delete. Nine storage tests plus the live run.

### Session management (2026-09-26)

`GET /api/user/sessions`, `DELETE /api/user/sessions/:sid` and
`POST /api/user/sessions/revoke-others`, matching the reference
(`router/api-router.go:94-96`). There was previously no way to see or end a
session short of signing out.

Two properties are deliberate rather than incidental:

- **Revocation is scoped by `(user_id, session_id)`.** `revoke_session_by_id` takes
  the owner as an argument instead of trusting a caller-supplied id, so one user
  cannot sign another out by naming their session. Keying on the id alone is the
  obvious implementation; removing the scope makes the test fail
  (mutation-verified).
- **The raw token is never echoed.** The list renders a `SessionView` with
  `id`/`created_at`/`expires_at`/`current` and no token field, so a live credential
  does not reach a response body or any log that captures one. The live check
  asserts the field is absent.

`revoke-others` keeps the calling session so the action does not sign the user out
mid-request; the current session is identified by the caller's own token.

Verified live: register, sign in twice, list both sessions with exactly one marked
current, revoke the other (its token then answers `401` on `/api/auth/me`), refuse
a foreign session id, and run `revoke-others` (count 1, caller still signed in, the
third token `401`). Nine storage tests cover the same ground.

### Access tokens (2026-09-26)

`GET`/`POST /api/user/token` (generate), `DELETE /api/user/token` (revoke) and
`GET /api/user/token/status`, matching the reference
(`router/api-router.go:102-105`, `controller/access_token.go`). A login session is
short-lived and belongs to a browser; an access token is a long-lived credential a
script can carry, and it is now accepted anywhere a session token is.

Shape and precedence:

- 29–32 alphanumeric characters, the reference's `GenerateRandomKey(29..32)` shape.
  Entropy comes from UUIDv4 bytes (CSPRNG-backed, 122 bits) rather than a weaker
  source, so the alphabet mapping does not narrow the entropy.
- A generated value is checked against existing tokens before it is handed out, so
  a collision is retried instead of silently giving one user another's credential.
- Regenerating **rotates**: a user has at most one token, and the previous value
  stops authenticating. Verified live — the old token answered `401` while the new
  one answered `200`.
- **The value is returned exactly once and never readable back.** `status` reports
  `enabled` and `created_at` only, and the live check asserts the token string does
  not appear in the status response. Recovering a lost token is impossible by
  design; it is rotated instead.
- Lookup joins on `status='active'`, so disabling an account ends its API access
  without a separate revocation step (test-asserted).

Verified live: status before generating, generate (length 32), status after (value
absent), authenticate `/api/auth/me` with the token, rotate and confirm the old
value `401`s, revoke, and confirm an unauthenticated generate is refused with
`401`. Eight storage tests cover the same ground.

### TOTP primitive (2026-09-26)

`crates/oxygenrouter-core/src/totp.rs` implements RFC 6238 with the parameters the
reference uses (HMAC-SHA1, 30-second period, six digits,
`common.GenerateTOTPSecret`). Built against the RFC rather than pulled from a crate
so those parameters are explicit.

**Verified against the RFC's own published vectors.** Appendix B prints 8-digit
codes for a known seed; `tests/totp.rs` asserts the generator reproduces all six
SHA1 rows exactly. That is stronger evidence than a self-consistent test, because
a wrong-but-internally-tidy implementation still disagrees with every
authenticator app.

Two corrections came out of writing that test, both worth recording:

1. A six-digit code is the RFC's eight-digit value taken **modulo 10⁶** — the
   *low* six digits, not the leading six. Taking `&expected[..6]` yields `942870`
   where the correct answer is `287082`. Confirmed against an independent
   from-spec implementation before changing anything.
2. `hotp_value` must return the raw 31-bit truncation **before** `mod 10^digits`.
   Folding the modulus in made the 8-digit assertion re-reduce an already-reduced
   value (`00287082`), which is exactly the bug the raw-function split exists to
   prevent.

Also covered: base32 encode/decode (round-trip, the RFC 4648 `foobar` vector,
padding and lower-case tolerance, alphabet rejection), unpadded 32-character
secrets from CSPRNG bytes, `otpauth://` URIs with percent-encoded labels, a ±1
step acceptance window that rejects a code two steps away, constant-time
comparison, and rejection of malformed input (empty, short, long, non-digit,
full-width digits) without panicking.

**Not yet wired to HTTP:** no `/api/user/2fa/*` endpoints, no login gating, no
backup codes, no lockout after repeated failures. The reference has all four
(`controller/twofa.go`, `common/totp.go`); this commit lands the verifiable
primitive, and the API surface is the next step.

### Test & build baseline (2026-09-25)
- `cargo test --workspace` → **193 passed / 0 failed**, zero build warnings
  (`billing` 102 unit + 3 oracle-differential, `relay` 15 unit + 20 adaptor-contract,
  `webui` 8 billing-store + 5 usage-mapping, `core` 5 unit + 5 config-compat)
- `npm run build` (tsc + vite) → **passes** (2,032 modules)
- `cargo check --workspace` → clean

### Routing & reliability (P3) — verified end to end

Selection and protection, in `crates/oxygenrouter-proxy/src/selection.rs` and
`limits.rs`:

| Capability | Behaviour |
|---|---|
| Priority-tiered failover | the attempt counter indexes the priority tiers; each retry steps down, and a tier whose members were all attempted falls through |
| Smoothing-weighted pick | within a tier, matching NewAPI: an all-zero weight set gets equal weight; an average below 10 amplifies weights by 100 |
| Auto-disable | N consecutive failures exclude a channel from selection; the counter resets on success |
| Rate limiting | fixed-window counters scoped by client token (else IP) |
| Concurrency ceiling | global in-flight limit with a drop-guard permit — **NewAPI does not enforce one** |

**Verified** against a live upstream with a dead primary (priority 10) and a
healthy backup (priority 1):

| Check | Evidence |
|---|---|
| Failover works | **5/5** requests `200` served by `backup`; before the fix every one returned `502` |
| Auto-disable engages | latency `10,129 ms` on the first request (tries the dead primary) then `~1,600 ms` once it is excluded |
| Failover is auditable | log row shows winner `backup` with `failed over: <primary-id>` |
| Concurrency ceiling | ceiling 2 with 6 concurrent requests → **exactly 2 × `200`, 4 × `429`** |
| Quota rejection | insufficient balance → **`403`** with the real balance, matching NewAPI (`billing_session.go`); `429` is reserved for rate limiting |

One correctness fix worth calling out: **failover did not work at all** before
this change. Exhaustion was judged on the candidate set *after* removing
already-tried channels, so a two-channel deployment gave up after one failure
instead of using its backup. Every request with a dead primary returned `502`.
Unit tests did not catch it because the selection tests exercised the tier walk
without exclusions; the end-to-end run did.

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

### Fixed defects (2026-09-26)

3. **A Claude client on an OpenAI channel received an empty answer.** The OpenAI
   adaptor appended the client's own path to the base URL, so `POST /v1/messages`
   was forwarded as `/v1/messages` — a route an OpenAI upstream does not serve in
   OpenAI shape — while carrying an *Anthropic* body. The reply was then handed to
   the OpenAI→Claude converter, which found no `choices` and returned a valid but
   empty content block. Fixed by routing such requests to
   `{base}/v1/chat/completions` and translating the request body
   (`convert/claude_to_openai_request.rs`). Two earlier checks missed it because
   they asserted shape only; the new guard asserts the text.
4. **`stream_options.include_usage` was never set.** Now added on every streamed
   request, so the upstream's terminal usage frame exists and the request bills
   its real token count instead of zero.

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

### Shared by every OpenAI-compatible channel (2026-09-26)

The dialect handling described above originally lived in `OpenAiAdaptor` alone,
which meant the same defect remained reachable on the other channels that speak
OpenAI: `Ollama`, `AdvancedCustom`, and every `ApiType::OpenAi` fallback
(OpenRouter, DeepSeek, vLLM, SGLang, LiteLLM …). Copying the logic into each
adaptor would have invited it back, so it now lives once, in
`crates/oxygenrouter-relay/src/adapters/openai_compat.rs`, and the three adaptors
delegate to it: `request_url`, `convert_request`, `rewrite_model`,
`convert_response`.

Verified live against the reference instance, one channel per adaptor type:

| Channel provider | Claude client | Gemini client | OpenAI client |
|---|---|---|---|
| `openai` | `'PING'` | `'PING'` | `'PING'` |
| `ollama` | `'PING'` | `'PING'` | `'PING'` |
| `58` (`advanced_custom`, `{action}` template) | `'PING'` | `'PING'` | — |

The guard is registry-driven rather than a hand-written list:
`tests/all_openai_compatible_adaptors_share_dialect_support.rs` iterates the
adaptors and asserts the **exact** upstream URL each produces (not merely that
`/v1/messages` is absent — that would pass on a wrong-but-different URL), plus
translation and response shaping in both native dialects. A new
OpenAI-compatible adaptor is covered as soon as it is registered. Reverting
`Ollama`'s `request_url` or `convert_request` to the old verbatim behaviour makes
it fail (mutation-verified).

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

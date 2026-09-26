# Changelog

All notable changes to OxygenRouter are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added (P5 — sessions)
- **Session management** — `GET /api/user/sessions`,
  `DELETE /api/user/sessions/:sid`, `POST /api/user/sessions/revoke-others`,
  matching the reference (`router/api-router.go:94-96`). Previously there was no
  way to see or end a session short of signing out.

### Verified (P5 — sessions)
- Revocation is scoped by `(user_id, session_id)`: `revoke_session_by_id` takes the
  owner as an argument rather than trusting a caller-supplied id, so one user
  cannot sign another out by naming their session. Keying on the id alone is the
  obvious implementation and makes the test fail (mutation-verified).
- The raw token is never echoed. The list renders `id`/`created_at`/`expires_at`/
  `current` and no token field, so a live credential cannot reach a response body
  or a log that captures one; the live check asserts the field is absent.
- Live end to end: register, sign in twice, list both with exactly one marked
  current, revoke the other (its token then answers `401` on `/api/auth/me`),
  refuse a foreign session id, and run `revoke-others` (one removed, caller still
  signed in, the third token `401`). Nine storage tests cover the same ground.
- `cargo test --workspace` → **322 passed / 0 failed**, zero warnings.

### Added (P8 — vendors)
- **Vendor registry** — `vendors` table plus the `/api/vendors` group (list,
  search, get, create, update, delete), matching the reference's paths
  (`router/api-router.go:376-386`). This subsystem was missing entirely; the
  reference keeps 44 rows.
- `model_count` is computed by a correlated subquery over `model_metadata`, not
  stored, matching `model/vendor_meta.go:15-25` (field tag `gorm:"-"`). The
  difference is observable: deleting a model with no other write lowers the count,
  which a stored column could not do.
- Duplicate vendor names are refused with a readable message rather than a raw
  SQLite constraint error, and a rename that collides with a *different* vendor's
  name is refused while renaming to one's own name is allowed.

### Verified (P8 — vendors)
- Live: create three vendors, refuse a duplicate, list in name order with counts,
  attach two models and observe OpenAI's count become 2 (the other two staying 0),
  substring search, rename, delete. Nine storage tests cover the same ground plus
  the derived-count invariant directly.
- `cargo test --workspace` → **313 passed / 0 failed**, zero warnings.

### Added (P4 — remaining relay endpoints)
- `POST /v1/responses/compact`. Only the documented compaction fields are
  forwarded (`model`, `input`, `instructions`, `previous_response_id`,
  `parallel_tool_calls`, `service_tier`, `prompt_cache_key`,
  `prompt_cache_options`, `prompt_cache_retention`); the Codex-parity extras
  (`tools`, `reasoning`, `text`) are dropped before the upstream call, matching
  `relay/responses_handler.go:23-39` and
  `dto/openai_responses_compaction_request.go:11-27`. A body that is not a JSON
  object passes through untouched so an unusual client gets the upstream's own
  error rather than a silent rewrite.
- `POST /v1/alpha/search` (Codex standalone web search), forwarding the raw body
  so unknown fields survive, matching `buildAlphaSearchRequestBody`. No charge is
  applied: the upstream returns no usage, our billing engine is driven by
  reported usage, and synthesising an amount would be a guess rather than parity.

### Verified (P4 — remaining relay endpoints)
- Both endpoints live against the reference instance on an OpenAI channel.
  `/v1/responses/compact` returns `200` with a Responses-shaped body from both
  the reference and this relay. `/v1/alpha/search` returns the same `500`
  `get_channel_failed` from both — the reference refuses it for a channel family
  that does not support it (`relay/alpha_search_handler.go:24-35`), and we
  forward that upstream answer rather than inventing one.
- Field-trimming is unit-tested both ways: the nine documented fields survive and
  the three Codex-parity fields do not, plus a non-object body is untouched.
- `cargo test --workspace` → **304 passed / 0 failed**, zero warnings.

### Added (P4 — model listing dialects)
- **`GET /v1/models` now answers in the client's own dialect, and the Gemini
  discovery routes exist.** Ours returned OpenAI shape to everyone, so an
  Anthropic or Gemini SDK could not parse the list; `GET /v1beta/models` — the
  Gemini client's service-discovery route — 404'd outright. The dialect is
  resolved from the credential headers the same way the reference does
  (`router/relay-router.go:25-43`): `x-api-key` + `anthropic-version` is
  Anthropic, `x-goog-api-key` or `?key=` is Gemini, anything else OpenAI.
  New `crates/oxygenrouter-webui/src/model_list.rs`; routes `GET /v1beta/models`
  and `GET /v1beta/openai/models` added.
- OpenAI items now carry `supported_endpoint_types`, the field that tells an SDK
  which dialects a model id may be called through.

### Verified (P4 — model listing dialects)
- Field names are an exact match against the live reference, checked by comparing
  key sets rather than by eye. OpenAI: `data`/`object`/`success`, items with
  `created`/`id`/`object`/`owned_by`/`supported_endpoint_types`. Anthropic:
  `data`/`first_id`/`has_more`/`last_id`, items with
  `created_at`/`display_name`/`id`/`type`. Gemini: `models`/`nextPageToken`, items
  with all thirteen fields the reference emits. `created_at` is RFC 3339
  (`2021-07-20T10:40:00Z`), not a unix integer — the type is what an Anthropic SDK
  validates. All three credential styles and both new routes verified live.
- The router test drives real requests through the real `Router`, because
  `/v1beta/models` sits beside the `/v1beta/models/*path` wildcard, which answers
  the exact path too — so a missing exact route returns `200` with the wrong body
  and a status check cannot see it. The guard asserts the shape instead. Removing
  either route, or bypassing the dialect detection, makes it fail (all three
  mutation-verified).
- `cargo test --workspace` → **302 passed / 0 failed**, zero warnings.

### Added (P4 — Gemini dialect)
- **A Gemini client can now use an OpenAI channel.** `convert/gemini_to_openai_request.rs`
  translates `generateContent` shape both ways — request (`contents[].role` `model`→
  `assistant`, `systemInstruction`→a leading `system` message, `thought: true` parts→
  `reasoning_content`, `inlineData`/`fileData`→`image_url`,
  `functionCall`/`functionResponse`→`tool_calls`/`tool` messages with the call id
  resolved by function name, `generationConfig`→sampling fields) and response
  (`reasoning_content`→a `thought` part, `tool_calls`→`functionCall` parts,
  `usage`→`usageMetadata` with reasoning taken out of the candidate count). The
  OpenAI adaptor aims such a request at `{base}/v1/chat/completions`. Previously
  there was no request converter at all, so a Gemini client on an OpenAI-compatible
  channel (vLLM, SGLang, DeepSeek) could not work.

### Fixed (P4 — passthrough streaming)
- **`relay_passthrough` hard-coded `stream: false`**, so every pass-through endpoint
  asked its upstream for a buffered reply and returned `application/json`. A Gemini
  client posting to `:streamGenerateContent` received one JSON object instead of SSE
  and could not parse it at all. Detection is now three-valued — a `stream: true`
  body, an `Accept: text/event-stream` header, or the `:streamGenerateContent`
  method name (Gemini selects streaming only through the URL).
- **`is_stream_request` let a broad `Accept` header override an explicit
  `stream: false`.** An SDK that always sends `Accept: text/event-stream` while
  asking for a buffered reply would be marked streaming, and its JSON body handed to
  the SSE parser. An explicit in-body flag now wins.

### Verified (P4 — Gemini)
- Live against the reference: a Gemini client on an OpenAI channel receives
  `parts:[{text:"PING"}]` / `finishReason STOP` / `usageMetadata` `10/2/12`
  non-streaming, and 18 Gemini-shaped SSE frames ending in `STOP` with
  `totalTokenCount 2025` streaming (it was one JSON object before the fix).
  Claude and Gemini clients both work on the same channel.
- Mutation check: removing either the URL guard or the request translation makes
  `openai_channel_serves_gemini_clients` fail, so both are load-bearing.
- `cargo test --workspace` → **281 passed / 0 failed**, zero warnings.

### Changed (P4 — shared dialect handling)
- **The Anthropic/Gemini dialect handling is now shared by every OpenAI-compatible
  channel.** It had lived in `OpenAiAdaptor` alone, which left the same defect
  reachable on `Ollama`, `AdvancedCustom` and the `ApiType::OpenAi` fallbacks
  (OpenRouter, DeepSeek, vLLM, SGLang, LiteLLM …): a native-dialect client's own
  path was forwarded verbatim to an upstream that does not serve it. Copying the
  logic per adaptor would have invited the bug back, so it moved once into
  `adapters/openai_compat.rs` and the three adaptors delegate to it
  (`request_url`, `convert_request`, `rewrite_model`, `convert_response`).

### Verified (P4 — shared dialect handling)
- Live against the reference, one channel per adaptor type — `openai`, `ollama`
  and `advanced_custom` (a `{action}` template) each serve an Anthropic client, a
  Gemini client and an OpenAI client, all returning `'PING'`.
- The guard is registry-driven, so a newly registered OpenAI-compatible adaptor is
  covered automatically: `tests/all_openai_compatible_adaptors_share_dialect_support.rs`
  asserts the **exact** upstream URL (not merely the absence of `/v1/messages`,
  which a wrong-but-different URL would satisfy) plus translation and response
  shaping in both native dialects. Reverting `Ollama` to the old behaviour makes it
  fail (mutation-verified, both the URL and the translation).
- `cargo test --workspace` → **290 passed / 0 failed**, zero warnings.

### Fixed (P4 — client dialect)
- **A Claude client routed to an OpenAI channel received an empty answer.** The OpenAI
  adaptor appended the client's path to the base URL, so `POST /v1/messages` was forwarded
  to `/v1/messages` — a route an OpenAI upstream does not serve in OpenAI shape — while
  carrying an Anthropic body. The Anthropic reply was then handed to the OpenAI→Claude
  converter, which found no `choices` and returned a well-formed but **empty** content
  block. Such a request is now aimed at `{base}/v1/chat/completions` and its body is
  translated by the new `convert/claude_to_openai_request.rs` (system hoisting,
  `stop_sequences`→`stop`, `tools[].input_schema`→`function.parameters`,
  `tool_use`→`tool_calls`, `tool_result`→`tool` messages with images kept off the tool
  message, `image`→`image_url` data URLs). The reference does the same: a Claude-format
  relay hard-codes the chat route (`relay/channel/openai/adaptor.go:180-184`).
  The two earlier checks missed this because they asserted *shape* — `type == "message"`
  and `content` being a list — and an empty answer satisfies both.
- **`stream_options.include_usage` was never set on streamed requests.** An OpenAI upstream
  omits the terminal usage frame without it, so the request settled at zero. Same defect
  class as the hard-coded `stream: false` fixed in P2.

### Verified (P4)
- All four client/channel quadrants end to end against the live NewAPI instance:
  OpenAI→`chat.completion` `'PING'` (usage 10/2/12); Claude client on an OpenAI channel →
  `content=[{"type":"text","text":"PING"}]` with Anthropic usage (input 10, output 2);
  Claude client on an Anthropic channel → pass-through; OpenAI client on an Anthropic
  channel → translated. Streaming through the OpenAI channel emits the full Anthropic
  event sequence (`message_start`, `content_block_start`, deltas, `content_block_stop`,
  `message_delta`, `message_stop`) with real text, and `tokens_used=2033` reaches the log
  row where it used to be `0`.
- Mutation check: removing the new URL guard makes
  `openai_channel_serves_claude_clients` fail, so the guard is load-bearing.
- `cargo test --workspace` → **264 passed / 0 failed**, zero warnings.

### Added
- **Priority-tiered failover, auto-disable, rate limiting, and a global concurrency ceiling** (P3).
  - The attempt counter indexes priority tiers, so the first attempt takes the best priority and
    each retry steps down; a tier whose members were all attempted falls through to the next.
  - Within a tier, smoothing-weighted random pick matching NewAPI: an all-zero weight set gets equal
    weight, and an average below 10 amplifies every weight by 100 so small weights still spread load.
  - Auto-disable after N consecutive failures, reset on success.
  - Fixed-window rate limiting scoped by client token, falling back to IP.
  - A global in-flight ceiling with a drop-guard permit. **NewAPI does not enforce one.**
  - Failed channels are recorded on the response and in the request log, so a failover is auditable
    rather than invisible.

### Fixed (P3)
- **Failover did not work at all.** Exhaustion was judged on the candidate set *after* removing
  already-tried channels, so a two-channel deployment gave up after one failure instead of using its
  backup — every request with a dead primary returned `502`. Exhaustion is now judged on the full
  candidate set, with exclusions applied inside each tier. Verified: 5/5 requests now succeed via the
  backup, and auto-disable shows as latency dropping from 10.1 s to 1.6 s.
- **Insufficient quota answered `429`.** NewAPI reserves `429` for rate limiting and answers
  insufficient quota with `403 Forbidden` (`billing_session.go`); we now match.
- The quota error printed a placeholder `available 0`; it reports the real balance, and says
  "unknown" rather than a misleading zero when no layer could read it.
- Model-map globbing anchoring was wrong (`4o*` matched `gpt-4o`, `*4o` matched `gpt-4o-mini`).

- **Billing is wired into the request path.** Pre-consume → settle → refund now runs on every relay
  request that carries a wallet-backed key: the wallet is debited, the key's usage moves, and
  `request_logs.tokens_used` is populated with the upstream's real token count (it was always `NULL`).
  A failed request refunds in full; the ledger reconciles exactly with the wallet balance.
- `api_keys.user_id` binds a key to the wallet that pays for its requests (idempotent migration, with
  `ApiKey::owned_by`); without it no request could be attributed.

### Changed
- The four handlers that re-implemented dispatch (`chat_completions`, `text_completions`,
  `embeddings`, `image_generations`) now delegate to the shared `dispatch_openai`, removing ~240
  duplicated lines and routing every endpoint through one billing-aware path.
- `dispatch_openai` detects streaming itself rather than taking a flag from each caller: a caller
  that forgot to forward `stream: true` would send a non-streaming upstream request whose SSE body
  the response converter then cannot parse. Streaming requests bill from their final SSE usage frame
  instead of being charged as empty non-streaming calls.

### Fixed
- **Streaming requests were charged zero and lost their content type.** `dispatch_openai` hard-coded
  `stream: false`, so an SSE request was sent non-streaming and its response was typed
  `application/json`. Found by end-to-end verification, not unit tests.
- The `Footer` option default leaked another project's watermark
  (`GitHub@NDBlockConnect | BlockConnect@StarsailsClover`); it now carries this repository's mark.

### Verified
- End-to-end against a live upstream: wallet `100,000,000 → 99,999,969` µ$ for one chat request,
  `used_micros 0 → 31`, `tokens_used = 30`, charge arithmetic `(14 + 16*3)/1e6*500000 = 31` ✓,
  ledger `sum == balance`, streaming `200 text/event-stream` with 26 frames and `tokens_used=38`,
  and a forced upstream failure leaving the wallet byte-identical.
- `cargo test --workspace` → **163 passed / 0 failed**, zero warnings.

### Added (earlier)
- **Billing & quota engine** — new crate `oxygenrouter-billing`.
  - Tiered billing expressions (`expr.rs`), the modern NewAPI pricing path: token variables with
    auto-exclusion, `tier`/`fixed`/`param`/`header`/`u`/`has`, math helpers, fixed-offset timezone
    functions, ternaries and the full operator set. `quota = round(expr_USD / 1e6 * QuotaPerUnit * group_ratio)`.
  - Classic ratio pricing (`chat_quota.rs`) with the ±1 minimum-charge rule.
  - `quota_math.rs`: int32 saturation, half-away-from-zero rounding, and the wider 2^53-1 wallet domain.
  - `estimator.rs`: per-vendor token estimation (CJK/math/URL/emoji aware) used for pre-consume, plus
    cl100k BPE counting when the `tiktoken` feature is enabled.
  - `session.rs`: reserve → settle → refund with an idempotent refund, trust bypass, playground mode,
    and the invariant that a settled session can never refund.
- **Differential parity harness** — `tests/fixtures/live_newapi_quota_oracle.json` is a verbatim
  export of every `tiered_expr` request in the live instance. `tests/live_oracle_diff.rs` replays
  all **15,728 requests across 33 distinct expressions**: **zero mismatches**, including tier
  (branch) selection. This is the evidence for the G6 parity gate.

### Added (P1 — adapter dispatch)
- **Provider adapter dispatch** — `Channel.provider` is now functional. New module
  `oxygenrouter-proxy/src/dispatch.rs` resolves a channel's `ApiType`, builds the matching
  adaptor from `oxygenrouter-relay`, translates the request into the provider's wire format, and
  translates the response (including SSE) back to the client, extracting billing usage. Pass-through
  providers use the same path via `RelayFormat::Raw`.
- Per-provider authentication, previously impossible: `RelayInfo` now carries `api_key`,
  `credential_raw`, and `request_path`. `Authorization: Bearer` (OpenAI, Cohere, Ollama, AdvancedCustom,
  Bedrock API-key mode), `x-api-key` (Anthropic), `x-goog-api-key` (Gemini),
  `api-key` + `?api-version` (Azure), Bearer + project/location path (Vertex), and SigV4 (Bedrock AKSK).
- `Adaptor::sign_request` hook so body-signing providers (AWS SigV4) run after the body is serialized.
- Real per-provider URL construction: Anthropic `/v1/messages`, Gemini/Vertex
  `:generateContent` / `:streamGenerateContent`, Azure `/openai/deployments/{model}/...`,
  Bedrock `/model/{id}/invoke[-with-response-stream]`.
- Retry backoff with jitter (exponential, capped), configurable, with a `none` mode that reproduces
  immediate retry.
- Regression tests: 20 adaptor contract tests (auth header + URL shape + credential parsing) and
  5 config-compatibility tests.
- `docs/research/NEWAPI_SUPERSET_ANALYSIS.md` — verified superset gap study.

### Changed
- `GET /v1/models` is now derived from the database (enabled channels' model lists plus the model
  registry) instead of a hard-coded list, so adding a channel immediately exposes its models.
- Retry classification and backoff now come from `oxygenrouter-relay::retry`.

### Fixed
- **`config.json` settings were discarded when the file was partial.** `AppSettings` lacked per-field
  serde defaults, so a hand-written config missing any field failed to deserialize and startup then
  overwrote it with stock defaults — losing settings such as a custom listen port. Every field now has
  a `#[serde(default = "...")]`, and startup refuses to write back a config it could not parse.
- AWS region parsing rejected three-segment AWS regions such as `ap-southeast-2`, silently falling
  back to `us-east-1`. Caught by the new adaptor contract tests.
- Bedrock region parsing rejected three-segment AWS regions such as `ap-southeast-2`, silently
  falling back to `us-east-1`.

### Removed
- 220 MiB of redundant `ho/` + `ho.zip` tree copies (verified byte-identical to HEAD, zero unique
  content; inventory retained at `.devlogs/p0_ho_inventory.txt`).

## [v0.1.0] — 2026-09-14

### Added
- **Information architecture overhaul** based on NewAPI: split `Dashboard` into two distinct pages
  - **Overview** (`/ui/overview`): welcome panel, base address, 4-step setup checklist, service tile grid
  - **Model Analytics** (`/ui/analytics`): time-bucket line chart, error overlay, per-model / per-channel horizontal bars, channel health cards, time-range selector
- **Dark + light theme** with persistent toggle in the top bar (`document.documentElement.dataset.theme`)
- **Sticky top bar** with breadcrumbs, language pill (EN / 中) and theme toggle
- **Sidebar refinement**: 256 px → 72 px collapse with persisted `localStorage`, active-link marker, hover/transition polish
- **Settings tabs** (General · Routing · Upstream · Retention · Danger) with local token rotation and danger zone
- **Channels page** card grid: status dot, search, show-disabled toggle, per-card test / copy / edit / delete
- **API Keys page** row cards: reveal-once masked keys, copy button, enabled/disabled badge
- **Routes page** card view for Model Maps and list view for Route Rules
- **Logs page** filterable list, 56 px row virtualization, expandable error rows, auto-refresh every 10 s
- **Animation system**: page-fade-in, click-ripple, skeleton shimmer, status-dot pulse, scrollbar thinness
- **Backend `GET /api/dashboard?time_range=1h|24h|7d`** returns `time_series`, `channel_perf`, and `recent_requests`
- **`DELETE /api/logs`** for danger-zone reset

### Improved
- `AppSettings` extended with `theme` and `language` (persisted in `config.json` + `settings` KV)
- `DashboardSnapshot` now includes `time_series` and `channel_perf` snapshots

### Polished
- Channel modal and settings form now use shared `.modal-content`, `.field-label`, and `.settings-section` styles
- Mobile drawer preserves the workspace navigation, language switcher, and local-mode indicator
- Dashboard refresh interval stays at 10 s; pull-to-refresh via Refresh button

## [v26.0-Alpha 1] — Initial alpha

### Added
- Single-binary CLI: `oxygenrouter.exe` (Windows) / `oxygenrouter` (Linux/macOS)
- SQLite-backed storage (bundled, no external deps)
- OpenAI-compatible proxy endpoints:
  - `POST /v1/chat/completions` (streaming + non-streaming)
  - `POST /v1/completions`
  - `POST /v1/embeddings`
  - `POST /v1/images/generations`
  - `GET  /v1/models`
- Channel rotation with priority + weighted random selection
- Error-aware failover: 401/403/429/5xx/network → switch channel
- `context_length_exceeded` automatic long-context fallback (e.g. `gpt-3.5-turbo` → `gpt-3.5-turbo-16k`)
- Model router with `auto` heuristic:
  - vision → `gpt-4o`
  - long context → `gpt-3.5-turbo-16k`
  - tools → `gpt-4o-mini`
  - default → `gpt-3.5-turbo`
  - image gen → `dall-e-3`, audio → `tts-1`, embeddings → `text-embedding-3-small`
- Per-channel model rewriting via Model Maps (glob patterns)
- Per-request log with channel, model, status, duration (auto-refresh in WebUI)
- WebUI (React 18 + Vite + Tailwind, OxygenOrigin dark theme):
  - Channels page (add/edit/delete/test)
  - API Keys page
  - Routes page (model maps + rules)
  - Logs page (auto-refresh every 10s)
  - Settings page (host/port/token/retry)
- Local auth token: client sends any `Bearer xxx` to `localhost:3001`; OxygenRouter forwards to upstream
- Windows console subsystem: double-click `.exe` opens terminal window
- Auto-open browser on first launch (configurable)
- Watermark: `GitHub@OxygenAILab | OxygenAILab@StarsailsClover`

### Known limitations
- Only OpenAI-compatible upstream
- No quota/billing/multi-user
- Simple glob pattern matching for model maps (no full regex)
- No HTTPS for local server

[Unreleased]: https://github.com/OxygenAILab/OxygenRouter/compare/v0.1.0...HEAD
[v0.1.0]: https://github.com/OxygenAILab/OxygenRouter/releases/tag/v0.1.0
[v26.0-Alpha.1]: https://github.com/OxygenAILab/OxygenRouter/releases/tag/v26.0-Alpha.1

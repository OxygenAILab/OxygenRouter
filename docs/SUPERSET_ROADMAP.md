# OxygenRouter — Superset Roadmap vs NewAPI

> GitHub@OxygenAILab | OxygenAILab@StarsailsClover
>
> Derived from a full read of NewAPI `Calcium-Ion/new-api` (208,258 LOC Go) against
> OxygenRouter (6,584 LOC Rust). This document is the dependency-ordered plan to go
> from "single-provider pass-through proxy" to a **superset** of NewAPI.

---

> ## ⚠️ CORRECTION NOTICE (2026-09-25)
>
> The baseline numbers below were measured against a **stale upstream**. They have been
> superseded by a verified re-measurement. For the authoritative figures, read
> **[`docs/research/NEWAPI_SUPERSET_ANALYSIS.md`](./research/NEWAPI_SUPERSET_ANALYSIS.md)**.
>
> | Claim in this document | Verified reality |
> |---|---|
> | Upstream `Calcium-Ion/new-api` | Repo has moved to **`QuantumNous/new-api`** (48,881 stars) |
> | 208,258 LOC Go | **217,437** LOC Go **+ 253,686** LOC frontend = **~471,100** total |
> | OxygenRouter 6,584 LOC | **~17,950** LOC (9,569 Rust + 8,381 TS/TSX) |
> | 23 relay endpoints | **50** resolved relay routes across 11 route groups |
> | "40 provider dirs" | Confirmed: **40** adapter directories |
> | 13 relay endpoints implemented | Confirmed: **13** (`proxy.rs`, all `ANY`) |
> | WebUI pages | NewAPI has **65** canonical client routes incl. a **40-section** admin settings workspace; OxygenRouter has **18** real pages (6 of them stubs) |
>
> Additionally: NewAPI is **AGPL-3.0** while OxygenRouter is **MIT**. NewAPI may be read as a
> behavioral specification only; its code must not be copied or transliterated into this
> repository. See §7.4 of the analysis document.
>
> The **phase ordering** (P1 adapters → P2 billing → P3 routing → P4 protocol) in this document
> remains sound and is retained; only the sizing and one repository identity were wrong.
>
> One key status change since this document was written: the §2.6 note "remaining for full
> wiring" is still accurate — `oxygenrouter-webui` **does not depend on** `oxygenrouter-relay`,
> so the 9 adapters are dead code in production. That wiring is the current top priority.

---

## 0. The Honest Gap

| Dimension | NewAPI | OxygenRouter | Ratio |
|---|---|---|---|
| Source LOC | 208,258 Go | 6,584 Rust | 32× |
| Upstream adapters | **40** provider dirs | **0** (pure OpenAI pass-through) | ∞ |
| Controller modules | ~105 files | 1 `api.rs` | 105× |
| Relay endpoints | 23 + MJ + tasks + video + realtime | 13 | ~3× |
| Protocol converters | OpenAI↔Claude↔Gemini↔Responses matrix | none | ∞ |
| Billing engine | full ratio/price/quota/settle | none (tokens_used always None) | ∞ |
| Auth | password+OAuth(6)+passkey+2FA+email | password only | ~10× |
| Payment providers | 5 (Stripe/Creem/Epay/Waffo/Pancake) | 1 manual | 5× |
| Async task/plugin system | JS plugins + polling + artifacts | none | ∞ |

**Root cause:** we built a *gateway shell* (channels, keys, logs, UI) but never built the
*two engines that make a gateway a gateway*:
1. **The protocol conversion engine** (provider adapters).
2. **The billing/quota engine** (pricing, token counting, settlement).

Everything else is secondary. Priority 1 and 2 below are non-negotiable.

---

## 1. Dependency Graph (why this order)

```
[P1 Provider Adapter Layer]─────────────┐
   │ trait Adaptor + registry            │
   │ OpenAI / Anthropic / Gemini /       │
   │ Bedrock / Vertex / Ollama / Cohere  │
   │ + SSE state machines                │
   ▼                                     │
[P2 Billing & Quota Engine]◄─────────────┘  (usage extraction needs adapters)
   │ ratio tables, token counting        │
   │ pre-consume / settle / refund       │
   │ quota reserve (atomic)              │
   ▼
[P3 Routing & Reliability]                (needs channels + billing)
   │ priority tiers, weighted LB         │
   │ affinity, auto-disable, retry       │
   │ rate limiting, concurrency          │
   ▼
[P4 Protocol Superset]                    (needs adapter layer P1)
   │ moderations, batches, files,        │
   │ realtime WS, codex, count_tokens,   │
   │ native /v1/messages inbound         │
   ▼
[P5 Auth & Identity]                      (independent, parallelizable)
   │ OAuth×6, passkey, TOTP, email,      │
   │ sessions, access tokens, RBAC       │
   ▼
[P6 Payments & Subscriptions]            (needs P2 billing + P5 auth)
   │ Stripe/Creem/Epay/Waffo/Pancake     │
   │ webhooks, orders, compliance        │
   ▼
[P7 Async Tasks & Plugins]               (needs P2+P3)
   │ task model, sub/poll/settle,        │
   │ JS plugins, midjourney, video       │
   ▼
[P8 Model Admin & Ops]                   (needs P1+P2)
   │ metadata sync, ratio sync,          │
   │ vendor meta, pricing config,        │
   │ perf metrics, rankings, checkin     │
```

---

## 2. P1 — Provider Adapter Layer *(highest leverage, do first)*

**Goal:** make `Channel.provider` actually do something. Translate between wire formats.

### 2.1 New crate: `oxygenrouter-relay`

```
crates/oxygenrouter-relay/
  src/
    lib.rs
    adaptor.rs          // `trait Adaptor`
    registry.rs         // ApiType -> Box<dyn Adaptor> factory
    value.rs            // `RelayValue` enum (replaces Go's `any`)
    channel_type.rs     // ChannelType + ApiType enums + mapping + fallback
    convert/
      mod.rs            // converter registry: (From,To) -> converter
      openai_chat.rs
      claude_messages.rs // to/from + streaming state machine
      gemini_chat.rs     // to/from + streaming
      responses.rs
    usage.rs            // dialect-preserving BillingUsage envelope
    sse/
      mod.rs
      claude_stream.rs  // content_block -> tool index de-holing
      gemini_stream.rs
    adapters/
      openai.rs
      anthropic.rs
      gemini.rs
      bedrock.rs
      vertex.rs
      ollama.rs
      cohere.rs
      azure.rs
      openrouter.rs
      deepseek.rs
      ...
```

### 2.2 The trait (Rust model of Go's `Adaptor`)

```rust
#[async_trait]
pub trait Adaptor: Send + Sync {
    fn init(&mut self, info: &RelayInfo);
    fn get_request_url(&self, info: &RelayInfo) -> Result<String>;
    fn setup_request_headers(&self, headers: &mut HeaderMap, info: &RelayInfo) -> Result<()>;
    fn convert_request(&self, from: RelayFormat, req: RelayValue, info: &RelayInfo)
        -> Result<RelayValue>;                       // one generic, format-aware
    async fn do_request(&self, body: Bytes, info: &RelayInfo) -> Result<UpstreamResponse>;
    async fn do_response(&self, resp: UpstreamResponse, info: &RelayInfo)
        -> Result<(RelayValue, Usage)>;              // returns usage for billing
    fn get_model_list(&self) -> Vec<String>;
    fn get_channel_name(&self) -> &'static str;
}
```

**Key difference from Go:** Go uses `any` + type assertion. Rust should use a tagged
`RelayValue` enum so the converter registry is exhaustive-matchable:

```rust
pub enum RelayValue {
    OpenAiChat(ChatRequest),
    Claude(ClaudeMessagesRequest),
    Gemini(GeminiChatRequest),
    Responses(ResponsesRequest),
    Raw(serde_json::Value),   // fall-through / unknown
}
```

### 2.3 Two-tier channel identity

- `ChannelType` (DB, sparse, NewAPI-compatible numbering 0..64).
- `ApiType` (contiguous, internal, drives adaptor selection).
- `channel_type -> api_type` mapping with **fallback to OpenAIA** for unknown
  (this is why arbitrary OpenAI-compatible providers "just work" in NewAPI).

### 2.4 Converter registry

```
(directed graph)  OpenAIChat <-> Claude <-> Gemini <-> Responses
```
- `HashMap<(RelayFormat, RelayFormat), ConverterSpec>` where `ConverterSpec` has
  request / response / stream variants.
- Multi-step path expansion for pairs with no direct converter.

### 2.5 SSE state machines (the subtle part)

- **Claude → OpenAI**: content-block index → dense tool-call index de-holing;
  `message_start`/`content_block_delta`/`message_delta`/`message_stop` dispatch;
  `thinking_delta` → `reasoning_content`; patch missing usage onto `message_delta`.
- **Gemini → OpenAI**: `candidates[]` → choices; `inlineData` → markdown images;
  `functionCall` → tool_calls; STOP → isStop; usage merge across frames.
- **Bedrock**: use `aws-sigv4` (Rust) or `aws-sdk-bedrockruntime`; InvokeModel
  (not Converse) with Anthropic body for Claude models, messages-v1 for Nova.

### 2.6 Milestones

| M | Deliverable | Definition of done |
|---|---|---|
| P1.1 | `oxygenrouter-relay` crate + trait + registry + `RelayValue` | ✅ **DONE** — compiles, 0 errors |
| P1.2 | OpenAI adapter (extract from existing proxy) | ✅ **DONE** — pass-through + usage extraction |
| P1.3 | Anthropic adapter + Claude→OpenAI (req+resp+stream) | ✅ **DONE** — 15 unit tests incl. SSE tool-index de-holing, thinking→reasoning_content, cache usage |
| P1.4 | Gemini adapter + conversion | ✅ **DONE** — generateContent/streamGenerateContent + SSE |
| P1.5 | Bedrock adapter (SigV4) | ✅ **DONE** — hand-rolled SigV4 (tested), InvokeModel Anthropic body, credential parsing (2/3-part), cross-region prefixing |
| P1.6 | Vertex adapter | ✅ **DONE** — Gemini shapes + credential parsing |
| P1.7 | Ollama adapter | ✅ **DONE** — OpenAI-shim path |
| P1.8 | Remaining adapters (Cohere ✅, Azure ✅, AdvancedCustom ✅) | ✅ **DONE** — 9 adapters total |

**P1 STATUS: COMPLETE (structure).** 9 provider adaptors, 2 full conversion
matrices (OpenAI↔Claude, OpenAI↔Gemini), 3 SSE state machines, SigV4 signer.
20/20 workspace tests pass.

**Remaining for full wiring (P1.x continuation):**
- Wire `registry::get_adaptor` into `oxygenrouter-webui/src/proxy.rs` request path
  (currently the old `UpstreamClient` is still used).
- Pass provider-specific auth: `x-api-key` (Anthropic), `x-goog-api-key` (Gemini),
  SigV4 (Bedrock AKSK) — adaptors expose `setup_headers`; the caller must merge
  the channel key.
- Multi-key selection should happen before adaptor construction and be passed
  via `RelayInfo`.
- `/v1/models` dynamic list from DB `model_list` + adaptor `model_list()`.

---

## 3. P2 — Billing & Quota Engine *(the lifeblood)*

**Goal:** real token counting, per-model pricing, atomic quota reservation,
pre-consume → settle → refund. Currently `tokens_used` is always `None` and
key usage is always `0` — billing is completely inert.

### 3.1 Pricing model (NewAPI-compatible)

```
QuotaPerUnit = 500_000.0          // quota units per $1 USD
model_ratio = 1  <=>  $0.002 / 1K tokens  <=>  $2 / 1M

# ratio (per-token) model:
ratio       = model_ratio * group_ratio
promptQ     = (prompt - cached - cache_new - image) + cached*cache_ratio
              + cache_new*create_cache_ratio + image*image_ratio
completionQ = completion_tokens * completion_ratio
quota       = round( (promptQ + completionQ) * ratio )      // + audio + tool
if ratio != 0 && quota <= 0 { quota = 1 }
quota       = saturate_i32(quota)

# fixed-price (per-call) model:
quota = round( model_price * QuotaPerUnit * group_ratio )

# pre-consume estimate:
preTokens = max(prompt_tokens, 500) + max_tokens
preQuota  = truncate(preTokens * model_ratio * group_ratio)

# settle:
delta = actual - preConsumed
delta > 0 => debit delta ;  delta < 0 => refund -delta
```

### 3.2 New crate: `oxygenrouter-billing`

```
crates/oxygenrouter-billing/
  src/
    lib.rs
    ratio.rs          // model_ratio / completion_ratio / cache_ratio tables + JSON load
    group.rs          // group_ratio + group->group ratio
    pricing.rs        // PriceData snapshot passed through a request
    quota.rs          // compute_chat_quota / compute_audio / compute_per_call
    tokenizer.rs      // tiktoken-rs (cl100k_base) + per-model codec cache
    estimator.rs      // heuristic estimator for non-OpenAI (CJK-aware)
    count.rs          // CountRequestToken: text + tools + messages + media
    reserve.rs        // atomic TryReserve user/token quota
    session.rs        // BillingSession: pre_consume / settle / refund
```

### 3.3 Ratio tables (data, not code)

- Port NewAPI's `defaultModelRatio` (~200 entries), `defaultCompletionRatio`,
  `defaultModelPrice`, cache ratios, audio/image ratios as embedded JSON.
- Admin-editable full-map replace semantics (matching NewAPI's
  `UpdateModelRatioByJSONString`, which **replaces**, never merges).
- `FormatMatchingModelName` normalization (gizmo wildcards, gemini thinking variants).
- `getHardcodedCompletionModelRatio` prefix table + `locked` flag.

### 3.4 Token counting

- `tiktoken-rs` for OpenAI models (cl100k_base, per-model codec cache).
- Heuristic estimator for Claude/Gemini (CJK/math/URL aware).
- Media token counts: image (520 or tile math for gpt-4o), audio (256 or
  duration-based `ceil(sec)/60 * 1000`), video (8192), file (4096).

### 3.5 Atomic quota reservation

- SQLite: conditional `UPDATE ... WHERE quota >= ?` (returns rows_affected).
- Optional Redis + Lua when present.
- Two balances: **user wallet** (may go negative on settle = debt) and
  **token remain_quota** (hard floor).
- Trust bypass: if wallet > 10*QuotaPerUnit and token trusted/unlimited, skip
  pre-deduction entirely.

### 3.6 Settlement (BillingSession)

```
pre_consume:  token first -> funding second; rollback token on funding failure
settle:       delta = actual - pre; token adjust + wallet adjust
refund:       idempotent, async, only if not already settled
```

### 3.7 Billing log fields

`prompt_tokens`, `completion_tokens`, `quota`, `is_stream` as real columns;
`model_ratio`, `group_ratio`, `completion_ratio`, `cache_ratio`, `model_price`,
`user_group_ratio` inside an `other` JSON blob.

### 3.8 Milestones

| M | Deliverable | DoD |
|---|---|---|
| P2.1 | ratio tables + PriceData + lookup | `GetModelRatioOrPrice` parity |
| P2.2 | tokenizer (tiktoken-rs) + estimator + CountRequestToken | matches NewAPI counts ±0 for OpenAI models |
| P2.3 | quota compute (chat/audio/per-call) | unit tests against hand-computed values |
| P2.4 | atomic reserve (user+token) | concurrent test: no over-spend |
| P2.5 | BillingSession pre/settle/refund | failed request refunds exactly |
| P2.6 | wire usage extraction into P1 adapters | `tokens_used` populated from real responses |

---

## 4. P3 — Routing & Reliability

**Goal:** priority tiers, weighted LB, affinity, auto-disable, retry, rate limits.

### 4.1 Channel selection
- In-memory cache `group -> model -> [channel_ids]` sorted by priority DESC,
  rebuilt every 60s + immediate negative-cache on disable.
- `GetRandomSatisfiedChannel(group, model, retry, filters)`:
  - `retry` selects priority tier (clamped to lowest).
  - weighted random with smoothing (adjustment 100 when all-zero; factor 100 when avg<10).
- Auto-group iteration; cross-group retry when token allows.

### 4.2 Priority semantics
Priority is **not** an automatic cascade — the caller's `retry` counter walks tiers.
With `RetryTimes = 0` (NewAPI default) there is **no** fail-through. We should
default `max_retries > 0` to actually get multi-channel resilience (an improvement).

### 4.3 Channel affinity
- Hybrid LRU+Redis cache, namespace `channel_affinity:v1`, cap 100k, TTL 3600s.
- Key sources: `context_int`, `context_string`, `request_header`, `gjson` body path.
- Rules match on `model_regex` + optional `path_regex` / `user_agent_include` / `value_regex`.
- On success, pin channel; configurable skip-retry-on-failure.

### 4.4 Retry & auto-disable
- Retry loop: `for retry in 0..=max_retries`, re-select channel, dispatch, on error
  `processChannelError` then `shouldRetry`.
- Retryable ranges: 1xx, 3xx, 401-407, 409-499, 500-503, 505-523, 525-599.
- Always-skip: 504, 524. Auto-disable status: 401 (when enabled).
- Channel statuses: 0 unknown / 1 enabled / 2 manual / 3 auto-disabled.
- Multi-key rotation: `random` | `polling`; per-key status map; all-keys-disabled → channel disable.
- **Improvement over NewAPI:** add configurable exponential backoff (NewAPI has none).

### 4.5 Rate limiting
- Fixed-window (Redis Lua) or in-memory sliding-window-log fallback.
- Scopes: global-web (120/180s), global-api (360/180s), critical (20/1200s),
  critical-per-user, download/upload (10/60s), search-per-user (10/60s).
- Per-model/per-group limit (`ModelRequestRateLimit`): total + success counts,
  duration window; per-group override.

### 4.6 Concurrency & timeouts
- Global concurrency semaphore (NewAPI *lacks* this — we add it, wirable to
  `max_concurrent_requests`).
- Stream idle timeout (default 300s, reset per SSE line); scanner max buffer 128MB.
- Header timeout 1800s; idle conn 90s.

### 4.7 Milestones

| M | Deliverable |
|---|---|
| P3.1 | channel cache + selection (priority tiers + weighted LB) |
| P3.2 | retry loop + error classification + backoff (NewAPI+) |
| P3.3 | auto-disable + multi-key rotation + channel status |
| P3.4 | channel affinity (HybridCache) |
| P3.5 | rate limiting (fixed-window + memory fallback, all scopes) |
| P3.6 | global concurrency semaphore + stream/idle timeouts |

---

## 5. P4 — Protocol Superset

Add missing endpoints on top of P1. Status as of `7f553df`:

**Landed**
- ✅ `POST /v1/moderations`
- ✅ `POST /v1/batches` + `/v1/files` (upload/list/get/content/delete)
- ✅ `POST /v1/messages/count_tokens`
- ✅ Native inbound `/v1/messages` — and it now *works*: see the dialect fix in
  `docs/FACT.md`, where a Claude client on an OpenAI channel received an empty
  answer because the request was aimed at the wrong route.
- ✅ `/v1/engines/:model/embeddings`
- ✅ `/v1/models` is dynamic (DB `model_list` + `model_metadata`), plus
  `GET`/`DELETE /v1/models/:model`
- ✅ `POST /v1/fine_tuning/jobs` (+ `:id`, `:id/cancel`, `:id/events`), `/v1/edits`

**Remaining**
- ❌ `GET /v1/responses` WebSocket (Responses streaming session)
- ❌ `GET /v1/realtime` WebSocket (Realtime API)
- ✅ Native inbound `/v1beta/models/*path` now works against a plain OpenAI
  channel too: `convert/gemini_to_openai_request.rs` translates the request and
  the adaptor aims it at `{base}/v1/chat/completions`, both directions, streaming
  included. Verified live. (Was listed here as a gap; now closed.)
- ❌ Codex credential refresh + usage endpoints
- ❌ vLLM / SGLang per-channel metrics
- ❌ `/mj/*` Midjourney action set, `/v1/tasks/*`, `/v1/video/*`, `/pg/chat/completions`
  (reference route list: `router/relay-router.go`, `router/task-router.go`,
  `router/video-router.go`)

**Dialect handling is shared, not per-adaptor.** `adapters/openai_compat.rs` holds
the Anthropic/Gemini re-routing and translation once, and `OpenAiAdaptor`,
`OllamaAdaptor` and `AdvancedCustomAdaptor` all delegate to it, so an
OpenAI-compatible channel added later inherits it rather than repeating the bug.
`tests/all_openai_compatible_adaptors_share_dialect_support.rs` enforces this off
the registry. Verified live on `openai`, `ollama` and `advanced_custom` channels
with all three client dialects.

---

## 6. P5 — Auth & Identity

- ✅ **Sessions**: multi-session list/revoke/revoke-others landed
  (`/api/user/sessions`). An auth-version fence (invalidate all sessions on a
  credential change) is **not** implemented.
- ✅ **Access tokens**: generate/status/revoke landed
  (`/api/user/token`, `/api/user/token/status`), and a token authenticates the
  user API. Security-proof gating on generate/revoke is **not** implemented.
- **OAuth providers**: GitHub, Discord, LinuxDO, OIDC, Telegram, WeChat, + custom
  generic providers (DB-driven, access-policy engine).
- **Passkey / WebAuthn**: register/verify/delete, discoverable login, step-up verify.
- ✅ **TOTP 2FA**: RFC 6238 primitive verified against the RFC's published vectors
  (`core/totp.rs`), and wired to `/api/user/2fa/{status,setup,enable,disable,backup_codes}`
  with login gating and single-use recovery codes. Lockout after repeated
  failures and the security-proof step-up on enable/disable are **not** done.
- **Email**: binding (start/resend/confirm), verification send, password reset.
- **Universal security verification**: scope+context proof gate for sensitive ops.
- 🔶 **RBAC**: the permission catalog and role baselines are implemented and
  served at `/api/authz/catalog`, matching the reference's three resources
  (`channel`/`audit`/`task_plugin`) and two roles. **Enforcement is not**: the
  reference evaluates these through Casbin on every admin route; every admin
  route here still checks only the coarse `role == admin`. Audit logging of
  administrative operations is also not implemented.
- **Turnstile** on register/login/checkin/verification.

---

## 7. P6 — Payments & Subscriptions

**Landed**
- ✅ Plans CRUD + user purchase (balance pay) + the admin lifecycle
  (`/api/subscription/admin/{bind,users/:id/subscriptions,plans/:id/subscriptions,
  user_subscriptions/:id}`, invalidate/reset/delete). Granting does not charge,
  matching `AdminBindSubscription`.
- ✅ Manual payment orders, redemption codes (single-use + max-use).

**Remaining**
- ❌ **Providers**: Stripe, Creem, Epay, Waffo, Waffo Pancake.
- ❌ **Webhooks**: signature verification per provider; rate-limited; idempotent credit.
- ❌ Order locking by trade-no.
- ❌ Per-plan quota grant: `plan.quota_micros` is stored but never credited, so a
  purchased plan currently confers no spendable allowance. The reference credits
  the user (`CreateUserSubscriptionFromPlanTx`). This is the largest functional
  hole in the subscription feature.
- ❌ Per-plan purchase limits (`MaxPurchasePerUser`) and renewal/reset cycles.
- **Compliance**: `payment_compliance` gating (dashboard-session only).
- **Return paths**: validated redirect URLs.
- **Redemption**: codes with single-user-once + max-use (already partially done).

---

## 8. P7 — Async Tasks & Plugins

- **Task model**: `TaskID` + `Platform`, status/progress, properties, private data,
  artifacts.
- **Submit/poll/settle**: reserve quota → submit → persist → poll (15s) → settle/refund.
- **JS plugins**: sobek-equivalent runtime in Rust (e.g. `boa` or `rquickjs`),
  factory + DB-override layers, hot-reload, routing generations, public routes,
  host protocols (`openai_responses`, `openai_video`).
- **Midjourney**: full `/mj/submit/*` action set + task fetch.
- **Video**: `/v1/video/generations`, remix.
- **Artifacts**: token-or-artifact-access auth, media proxy with allowlists.

---

## 9. P8 — Model Admin & Ops

- **Model metadata CRUD** (exists) + **official sync** from `basellm.github.io/llm-metadata`
  (ETag/version-guarded, i18n zh/en/ja) + missing-models view + square state.
- ✅ **Vendor metadata** CRUD (list/search/get/create/update/delete on
  `/api/vendors`, with a derived `model_count`). Merge/multi-select operations
  (`/api/vendors/operations`) are not implemented.
- **Pricing config**: snapshot/preview/convert/batch-update/reset.
- **Ratio sync**: from official preset / models.dev / channels (OpenRouter + models.dev
  conversion).
- **Perf metrics**: latency/throughput by model/group/hour.
- **Rankings**: usage snapshot by period.
- **Check-in**: daily quota award.
- **Prefill groups**, **group management** (all/usable/groups+ratios).
- **Uptime-Kuma** integration.
- **System info**: instances, stale-instance cleanup, system tasks (scheduled).

---

## 10. Anti-Goals / Deliberate Improvements

Things where we should **not** blindly copy NewAPI:

| Area | NewAPI | OxygenRouter |
|---|---|---|
| Retry backoff | none (immediate) | **exponential backoff + jitter** |
| Global concurrency | not enforced | **real semaphore** |
| Auto-disable default | off | **configurable, on for 401/5xx** |
| Response cache | none | **optional prompt/result cache** |
| API-key update | create/delete only | **full PUT update** |
| `/v1/models` | static per channel-type | **dynamic from DB** |

---

## 11. Execution Order (single track)

```
P1.1 registry ──► P1.2 openai ──► P1.3 anthropic ──► P1.4 gemini
   │                                  │
   │                                  ▼
   └──────────────► P2.1 ratios ──► P2.2 tokenizer ──► P2.3 quota
                        │                                │
                        ▼                                ▼
                    P2.4 reserve ──► P2.5 session ──► P2.6 usage-wire
                                                          │
                                                          ▼
                                          P3 routing/reliability (1..6)
                                                          │
                          ┌───────────────────────────────┼───────────────┐
                          ▼                               ▼               ▼
                    P4 superset                    P5 auth         P8 model admin
                          │                               │
                          ▼                               ▼
                    P7 tasks/plugins               P6 payments
```

**Start now with P1.1** (trait + registry + `RelayValue`), then P1.2 (extract the
existing OpenAI pass-through into the trait), then P1.3 (Anthropic — the single
highest-value adapter since Claude is the most-used non-OpenAI provider).

---

*End of roadmap.*

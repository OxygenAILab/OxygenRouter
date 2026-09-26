<!-- Gi   t  H   ub  @  Ox   y   g enAILab | Oxyge  n  AI  La b@St ars  a  il sCl  o  v  er -->

# NewAPI Superset Analysis — Verified Gap Study

> **GitHub@OxygenAILab | OxygenAILab@StarsailsClover**

This document is the evidence base for the goal: **OxygenRouter must become a full functional
superset of NewAPI, with a rational re-arrangement, and must be demonstrably stronger, backed by
real usage comparison and research.**

Every number here was measured from primary sources on 2026-09-25. Where a claim comes from a
source file, the path is given. Nothing is estimated from memory.

---

## 0. Method & Primary Sources

| # | Source | Identity | How used |
|---|---|---|---|
| S1 | `C:\Users\Sails\Documents\Workspace\02-Research\new-api-upstream` | Upstream NewAPI `QuantumNous/new-api` @ `c2b7a9a`, shallow clone | Authoritative page/route/engine enumeration |
| S2 | `C:\Users\Sails\Documents\Default Project\new-api-v1.0.0-rc.37.exe` (running, PID 3192, `:3000`) | Live NewAPI **v1.0.0-rc.37** instance | Live behavior probing, `/api/status` etc. |
| S3 | `C:\Users\Sails\Documents\Default Project\one-api.db` (21.4 MB, written 2026-09-25) | Live NewAPI SQLite database | Real schema (38 tables) + real data volumes |
| S4 | OxygenRouter working tree @ `aee549b` | Our implementation | Gap measurement |
| S5 | `C:\Users\Sails\.codex\config.toml` | Host agent config | Note: the host agent's own model provider is `NewAPI` at `localhost:3000` |

### Critical corrections to prior project assumptions

The pre-existing `docs/SUPERSET_ROADMAP.md` contains **three material factual errors** that must be
corrected before further planning, because they mis-size the target:

| Prior claim (SUPERSET_ROADMAP.md) | Verified reality | Impact |
|---|---|---|
| Upstream = `Calcium-Ion/new-api`, 208,258 LOC Go | Canonical repo is **`QuantumNous/new-api`** (48,881 stars, `default_branch=main`, 76 MB), and the total is **~471k LOC** (217,437 Go + 253,686 frontend) | Target is **2.3× larger** than assumed |
| "40 upstream adapters" / "23 relay endpoints" | **40 adapter dirs** confirmed; relay endpoints are **50 resolved routes across 11 route groups**, not 23 | Endpoint target understated 2.2× |
| Pages: 5 WebUI pages | NewAPI has **65 canonical client routes** (73 typed route entries; group segments `(auth)`/`(errors)`/`_authenticated` stripped, `$section` children collapsed), incl. a **40-section** admin settings workspace | Page target understated 13× |

Also: NewAPI is **AGPL-3.0** (`new-api-upstream/LICENSE`), while OxygenRouter is **MIT**
(`Cargo.toml`). Any porting of NewAPI code must respect copyleft. This study therefore treats
NewAPI strictly as a **behavioral specification**, and all OxygenRouter implementation must be
original. See §7 for the verification protocol that enforces this.

<!-- GitHub@O xygenAILab | Oxyge nAILab@ Star sa ils Clover -->

---

## 1. Scale Comparison (measured)

| Metric | NewAPI | OxygenRouter | Ratio |
|---|---|---|---|
| Backend LOC | **217,437** Go (1,009 files) | **9,569** Rust (45 files) | 22.7× |
| Frontend LOC | **253,686** TS/TSX (1,349 files) | **8,381** TS/TSX (41 files) | 30.3× |
| **Total LOC** | **~471,100** | **~17,950** | **26.2×** |
| HTTP routes (backend) | **328** | **73** (60 admin/user API + 13 relay) | 4.5× |
| Client routes (pages) | **65** canonical | **18** real pages (22 `<Route>`, 4 redirects) | 3.6× |
| Admin settings sections | **40** (8 groups) | **0** (flat `SystemSettingsPage`, 182 lines) | ∞ |
| Upstream adapters | **40** dirs | **9** (relay crate, **not wired**) | 4.4× |
| DB tables | **38** (from live DB) | **17** | 2.2× |
| i18n locales | **7** (en, zh, zh-TW, ja, fr, ru, vi) | **2** (en, zh) | 3.5× |
| i18n keys | **6,778** per locale | **~1,077** leaf keys | 6.3× |
| Middleware modules | **33** files | 0 (inline) | — |
| Service modules | **100+** files | 0 (inline in `api.rs`) | — |

### Frontend page reality check (OxygenRouter)

Measured line counts reveal that several declared pages are **placeholder stubs**:

| Page | Lines | Status |
|---|---|---|
| `AdminUsersPage.tsx` | 4 | **stub** |
| `SubscriptionsPage.tsx` | 4 | **stub** |
| `AdminBillingPage.tsx` | 5 | **stub** |
| `AuthPage.tsx` | 12 | **stub** |
| `PlansPage.tsx` | 18 | **stub** |
| `WalletPage.tsx` | 20 | **stub** |
| `AnalyticsPage.tsx` | 38 | thin (delegates) |
| — | | **6 real stubs of 18 pages** |

Evidence: `crates/oxygenrouter-webui/web/src/pages/*.tsx` line counts.

---

## 2. NewAPI Complete Page Inventory (verified)

Extracted from `web/src/routes/**` and `web/src/routeTree.gen.ts`.

### 2.1 Public / unauthenticated

| Route | Purpose |
|---|---|
| `/` | Public home (hero, features, how-it-works, stats, CTA) |
| `/about` | About page |
| `/pricing`, `/pricing/$modelId` | **Public model pricing catalog** + per-model detail (API info, performance) |
| `/rankings` | **Public rankings**: hero, model leaderboard, market share, pulse, models section |
| `/privacy-policy`, `/user-agreement` | Legal documents (admin-editable) |
| `/setup` | First-run initialization wizard (multi-step) |

### 2.2 Authentication

| Route | Purpose |
|---|---|
| `/sign-in`, `/sign-up`, `/register` | Credential auth |
| `/otp` | OTP second factor |
| `/forgot-password`, `/reset`, `/user/reset` | Password recovery |
| `/oauth`, `/oauth/$provider` | Generic OAuth callback (GitHub / Discord / OIDC / LinuxDO / Telegram / WeChat / custom) |

### 2.3 Authenticated console (`_authenticated/*`)

| Route | Sections | Purpose |
|---|---|---|
| `/dashboard` | `overview`, `models`, `flow`, `users`(admin) | Data dashboard |
| `/channels` | — | Channel management |
| `/keys` | — | API token management |
| `/models` | `metadata`, `vendors`, `deployments` | Model registry |
| `/playground` | — | Chat playground |
| `/chat/$chatId`, `/chat2link` | — | Preset chat + chat-to-link sharing |
| `/usage-logs` | `common`, `drawing`, `task` + `audit` | Logs |
| `/users` | — | User administration |
| `/wallet` | — | Top-up / balance / payment products |
| `/subscriptions` | — | Subscription plans & purchase |
| `/redemption-codes` | — | Redemption code admin |
| `/profile` | — | Personal profile + notification prefs |
| `/security` | — | 2FA / passkey / session security |
| `/system-info` | — | Instances, system tasks, maintenance |
| `/task-plugins` | — | JS task plugin marketplace |
| `/errors/$error` | — | Error routing |
| `/(errors)/401,403,404,500,503` | — | Dedicated error pages |

### 2.4 Admin System Settings — **40 sections in 8 groups**

Verified from `web/src/features/system-settings/*/section-registry.tsx`.

| Group | Sections (id → title) |
|---|---|
| **Site & Branding** (4) | `system-info` → System Information · `notice` → System Notice · `header-navigation` → Header navigation · `sidebar-modules` → Sidebar modules |
| **Authentication** (5) | `basic-auth` → Basic Authentication · `oauth` → OAuth Integrations · `passkey` → Passkey Authentication · `bot-protection` → Bot Protection · `custom-oauth` → Custom OAuth |
| **Billing & Payment** (6) | `quota` → Quota Settings · `currency` → Currency & Display · `model-pricing` → Model Pricing · `group-pricing` → Group Pricing · `payment` → Payment Gateway · `checkin` → Check-in Rewards |
| **Models** (5) | `global` → Global Model Configuration · `gemini` → Gemini · `claude` → Claude · `grok` → Grok · `model-deployment` → Model Deployment |
| **Request policies** (3) | `filtering` → Request checks · `routing` → Sessions and retries · `health` → Channel health |
| **Security & Limits** (3) | `rate-limit` → Rate Limiting · `ssrf` → SSRF Protection · `token-limits` → Token Limits |
| **Console Content** (7) | `dashboard` → Data Dashboard · `announcements` → Announcements · `api-info` → API Addresses · `faq` → FAQ · `uptime-kuma` → Uptime Kuma · `chat` → Chat Presets · `drawing` → Drawing |
| **Operations** (7) | `behavior` → System Behavior · `alerts` → Monitoring & Alerts · `email` → SMTP Email · `worker` → Worker Proxy · `logs` → Log Maintenance · `performance` → Performance · `update-checker` → System maintenance |

### 2.5 UI architecture patterns (notable, must be matched or exceeded)

| Pattern | Evidence | Note |
|---|---|---|
| Vercel-style **drill-in sidebar**: entering `/system-settings/*` replaces root nav with admin groups | `lib/sidebar-view-registry.ts`, `config/system-settings.config.ts` | OxygenRouter has flat 256/72 px sidebar only |
| **Section registry factory** shared by all settings groups | `utils/section-registry.ts` + 8 registries | Our settings are one 182-line page |
| **Runtime nav gating** from server status: `HeaderNavModules` (per-module `enabled`/`requireAuth`) and `SidebarModulesAdmin` | `lib/nav-modules.ts`; confirmed live in `/api/status` | Fully dynamic IA |
| **Responsive nav with gestures**: mobile bottom drawer w/ spring physics | `components/mobile-drawer.tsx`, `constants.ts` (`MOBILE_DRAWER_ANIMATION`) | |
| Command palette + route transition + `animate-in-view` | `components/`, `page-transition.tsx` | |
| TanStack Router (file-based, typed `routeTree.gen.ts`) | `routeTree.gen.ts` | We use `react-router-dom` v6 |

<!-- Oxy  gen  AI  Lab@  Star  sails  Clo  ver  -->

---

## 3. NewAPI Complete Backend Surface (verified)

### 3.1 Route counts

Extracted and **group-prefix-resolved** from `router/*.go` (comment-stripped, `Group()` prefix chaining resolved):

| Router file | Routes |
|---|---|
| `api-router.go` | 264 |
| `relay-router.go` | 50 |
| `task-router.go` | 5 |
| `video-router.go` | 3 |
| `dashboard.go` | 4 |
| `channel-router.go` | 1 |
| `authz-router.go` | 1 |
| **Total** | **328** |

By method: `GET` 146 · `POST` 137 · `DELETE` 28 · `PUT` 17 · `PATCH` 3.

### 3.2 Admin/user API groups (from `api-router.go`)

`/api` root: setup, status, uptime, models, notice, agreements, about, home content, pricing,
perf-metrics, rankings, verification, password reset.

Grouped sub-routers: `/api/user/*` (auth, sessions, tokens, passkey, 2FA, aff, topup, pay, checkin,
oauth bindings, self, admin CRUD) · `/api/subscription/*` (+ `/admin`) · `/api/option/*` (settings,
request_policy, model_pricing, payment_compliance, channel_affinity_cache, waffo-pancake) ·
`/api/custom-oauth-provider/*` · `/api/performance/*` · `/api/ratio_sync/*` · `/api/plugin/task/*`
(incl. marketplace, versions, activate, dryrun) · `/api/token/*` (batch, auto-groups, key reveal) ·
`/api/redemption/*` · `/api/log/*` · `/api/system-task/*` · `/api/system-info/*` (instances) ·
`/api/data/*` (flow analytics) · `/api/group/` · `/api/prefill_group/` · `/api/mj/` · `/api/task/` ·
`/api/vendors/*` · `/api/models/*` (sync_upstream, missing, delete) · `/api/deployments/*`
(GPU rental: hardware-types, locations, replicas, price-estimation, containers, logs, extend).

### 3.3 Relay protocol surface (50 resolved routes)

| Group | Endpoints |
|---|---|
| `/v1` OpenAI | `chat/completions`, `completions`, `embeddings`, `moderations`, `edits`, `rerank`, `images/variations`, `audio/{transcriptions,translations,speech}`, `files` (POST/GET/DELETE/`:id/content`), `fine-tunes` (+`:id/cancel`,`:id/events`), `models` (GET/POST/DELETE), `engines/:model/embeddings` |
| `/v1` Anthropic/Responses | `messages`, `messages/count_tokens`, `responses` (**WS**), `responses/compact`, `realtime` (**WS**), `alpha/search` |
| `/v1beta` Gemini | `models/*path` POST |
| `/v1/tasks` | `POST :key`, `GET :key`, `GET :key/artifacts`, `GET :key/artifacts/:artifact_key/content` |
| `/v1` video | `video/generations`, `video/generations/:task_id`, `videos/:video_id/remix` |
| `/mj` (Midjourney) | `submit/{action,shorten,modal,imagine,change,simple-change,describe,blend,edits,video,upload-discord-images}`, `notify`, `task/:id/fetch`, `task/:id/image-seed`, `task/list-by-condition`, `insight-face/swap` |
| `/pg` | Playground relay |

### 3.4 Engines & subsystems (the real moat)

| Subsystem | Evidence | Capability |
|---|---|---|
| **Provider adapters** | `relay/channel/` — **40** dirs: `openai, claude, gemini, aws, vertex, azure?, ollama, cohere, deepseek, xai, mistral, moonshot, zhipu, zhipu_4v, xunfei, baidu, baidu_v2, ali, tencent, volcengine, minimax, siliconflow, openrouter, perplexity, cloudflare, replicate, coze, dify, jina, lingyiwanwu, palm, ai360, mokaai, jimeng, sub2api, submodel, newapi, advancedcustom, codex, task` | 40 upstream protocols |
| **Protocol converters** | `service/{request,response}_converter.go`, `relaykit/relayconvert`, `pkg/` | OpenAI ↔ Claude ↔ Gemini ↔ Responses matrix |
| **Billing** | `service/{billing,billing_session,billing_usage,tiered_settle,text_quota,quota,image_billing,violation_fee,funding_source}.go`, `common/quota_math.go`, `pkg/billingexpr` | quota math, pre-consume/settle/refund, tiered pricing, expression DSL |
| **Tokenizer** | `service/{tokenizer,token_counter,token_estimator}.go` | tiktoken + estimation |
| **Channel selection & affinity** | `service/{channel_select,channel,channel_affinity}.go`, `middleware/distributor.go` (25 KB) | priority tiers, weighted LB, affinity cache, auto-groups |
| **Rate limiting** | `common/rate-limit.go`, `middleware/{rate-limit,model-rate-limit}.go` | global/critical/per-user/per-model scopes |
| **Auth full stack** | `middleware/auth.go` (18 KB), `service/{auth_session,auth_token,account_security,login_verification,twofa,security_verification}.go`, `service/passkey/`, `service/authz/`, `oauth/` | sessions, access tokens, passkey/WebAuthn, TOTP 2FA, email binding, RBAC/Casbin, OAuth providers |
| **Payments** | `service/{epay,waffo_pancake,webhook,return_path}.go`, `middleware/*webhook*` | Epay, Stripe, Creem, Waffo, Waffo Pancake |
| **Async tasks & JS plugins** | `service/{task,task_polling,task_billing,task_artifact_store,task_plugin_*}.go`, `pkg/jsplugin`, `plugins/tasks` (video providers: alibaba/doubao/google/hailuo/jimeng/kling/sora/suno/veo/vertex/vidu) | task lifecycle, artifacts, JS plugin runtime, marketplace |
| **Media** | `service/{image,audio,midjourney,file_service,download}.go`, `plugins/tasks/*` | image/audio/video generation + Midjourney |
| **Security** | `common/ssrf_protection.go`, `common/password_crypto.go`, `common/totp.go`, `middleware/{turnstile-check,trusted_proxies,cors,audit}.go` | SSRF, Turnstile, trusted proxies, audit log |
| **Ops** | `service/{system_task,system_instance,rankings,perf_metrics}.go`, `pkg/{perf_metrics,cachex,wsmanager}` | scheduled tasks, multi-instance, rankings, perf metrics, disk cache, WS manager |
| **Multi-DB** | `common/redis.go`, `common/disk_cache*.go`, `common/body_storage.go` | SQLite/MySQL/PostgreSQL + Redis + disk cache |

### 3.5 Live database schema (38 tables, from S3)

`abilities, audit_logs, auth_flows, authz_roles, casbin_rule, channels, checkins,
custom_oauth_providers, external_identity_claims, login_encryption_keys, logs, midjourneys, models,
options, passkey_credentials, perf_metrics, prefill_groups, quota_data, redemptions, setups,
subscription_orders, subscription_plans, subscription_pre_consume_records, system_instances,
system_task_locks, system_tasks, task_plugins, tasks, tokens, top_ups, two_fa_backup_codes, two_fas,
user_oauth_bindings, user_sessions, user_subscriptions, users, vendors`

Live data volumes (real usage, S3): `logs`=18,974 · `system_tasks`=1,715 · `quota_data`=554 ·
`models`=351 · `perf_metrics`=244 · `audit_logs`=250 · `vendors`=44 · `options`=38 keys ·
`user_sessions`=26 · `channels`=11 · `tokens`=4 · `users`=2 · `casbin_rule`=7 · `authz_roles`=2 ·
`two_fas`=1.

---

## 4. OxygenRouter Current State (verified)

### 4.1 Structure

| Crate | Contents |
|---|---|
| `oxygenrouter-core` | `models.rs`, `db.rs`, `config.rs`, `options.rs` |
| `oxygenrouter-proxy` | `client.rs`, `router.rs`, `scheduler.rs`, `tester.rs`, `upstream.rs` |
| `oxygenrouter-relay` | **9 adapters** + `convert/` (OpenAI↔Claude, OpenAI↔Gemini) + `sse/` (claude, gemini, openai) + `sigv4.rs`; 101 `pub` items; **15 tests pass** |
| `oxygenrouter-webui` | `api.rs` (1,766 lines, all admin/user handlers), `proxy.rs`, `embed.rs`, `state.rs` |
| `oxygenrouter-bin` | CLI entry |

### 4.2 Blocking defect: relay is not wired

`crates/oxygenrouter-webui/Cargo.toml` **does not list `oxygenrouter-relay` as a dependency**, and
`crates/oxygenrouter-webui/src/proxy.rs` contains **zero** references to `relay` / `registry` /
`get_adaptor`. Confirmed by grep. Therefore:

> The 9 provider adapters — the single highest-value asset for superset parity — are **dead code in
> production**. Requests still flow through the legacy OpenAI pass-through client.

This is the highest-priority fix in the whole program: it is a small wiring change that unlocks the
entire P1 investment, and it is the prerequisite for P2 (billing) and P4 (protocol superset).

### 4.3 Verified measurements

| Metric | Value |
|---|---|
| Rust LOC / files | 9,569 / 45 |
| TS LOC / files | 8,381 / 41 |
| Admin+user API routes | 60 (`api.rs`) |
| Relay routes | 13 (`proxy.rs`, all `ANY`) |
| DB tables | 17 (`channels, api_keys, model_maps, model_metadata, route_rules, request_logs, settings, migrations, users, auth_sessions, ledger_entries, subscription_plans, subscriptions, redemption_codes, redemption_uses, payment_orders`) |
| Backend tests | **20 pass / 0 fail** (`cargo test --workspace`) |
| Frontend build | **passes** (tsc + vite, 2,032 modules) |
| i18n | 2 locales, ~1,077 leaf keys |
| Pages implemented | 12 substantive + 6 stubs |

Residual value we already hold that NewAPI does **not** have:

- Single static Rust binary (NewAPI ships a 132 MB Go binary and needs MySQL/PG for scale)
- `relay` crate with clean `Adaptor` trait + `RelayValue` tagged enum (compile-time exhaustive
  converter dispatch; Go uses `any` + runtime assertions)
- Hand-rolled SigV4 with unit tests

---

## 5. Gap Matrix

Legend: **M** = missing entirely · **P** = partial · **S** = stub · **✓** = present.

### 5.1 Console IA

| NewAPI page | OxygenRouter | Gap |
|---|---|---|
| `/dashboard` overview | `OverviewPage` ✓ | — |
| `/dashboard` flow / models / users | `AnalyticsPage` (38 lines) **S** | M: 3 analytics sections |
| `/channels` | `ChannelsPage` ✓ | P: no affinity, no batch key, no inference status |
| `/keys` | `KeysPage` ✓ | P: no quota/limits/group/model restriction |
| `/models` metadata | `ModelMetadataPage` ✓ | P |
| `/models` vendors | — | **M** |
| `/models` deployments | — | **M** |
| `/usage-logs` common | `LogsPage` ✓ | P: no task/drawing/audit splits, no billing breakdown |
| `/usage-logs` task / drawing / audit | — | **M** |
| `/users` | `AdminUsersPage` (4 lines) **S** | **M** |
| `/wallet` | `WalletPage` (20 lines) **S** | **M** |
| `/subscriptions` | `SubscriptionsPage` (4) **S** | **M** |
| `/redemption-codes` | `AdminBillingPage` (5) **S** | **M** |
| `/profile` | — | **M** |
| `/security` (2FA/passkey/sessions) | — | **M** |
| `/system-info` | `SystemInfoPage` ✓ | P |
| `/task-plugins` | — | **M** |
| `/playground` | ✓ | P: no preset chat, no chat2link |
| `/pricing` (public) | `PlansPage` (18) **S** | **M** |
| `/rankings` (public) | — | **M** |
| `/about`, legal pages | — | **M** |
| `/setup` wizard | — | **M** |
| `/errors/*` (401/403/404/500/503) | — | **M** |
| auth: sign-in/up/register/otp/forgot/reset/oauth | `AuthPage` (12) **S** | **M** |
| **System Settings: 40 sections** | one 182-line flat page **S** | **M** |

### 5.2 Backend

| Subsystem | NewAPI | OxygenRouter | Gap |
|---|---|---|---|
| Provider adapters (runtime) | 40 | 0 wired (9 built) | **M** |
| Protocol converters (runtime) | full matrix | 0 wired | **M** |
| Billing / quota engine | full | none (`tokens_used` unused) | **M** |
| Tokenizer | tiktoken + estimator | none | **M** |
| Channel selection | priority tiers + weighted + affinity + auto-groups | basic priority+weight | P |
| Auto-disable / multi-key rotation | full | partial | P |
| Rate limiting | 6 scopes | none | **M** |
| SSRF protection | ✓ | none | **M** |
| Auth: sessions/access tokens/passkey/2FA/email/RBAC | full | password + sessions + access tokens + TOTP 2FA with login gating and recovery codes + a deny-by-default console gate (public/user/admin/root) | **P** (missing: passkey, email verification, per-permission enforcement, step-up proofs, lockout) |
| OAuth providers | GitHub/Discord/OIDC/LinuxDO/Telegram/WeChat/custom | none | **M** |
| Payments | 5 providers + webhooks | manual order only | **M** |
| Subscriptions | full (plans/orders/pre-consume) | plans, purchase, admin lifecycle, and a per-plan quota pool that funds requests via a `FundingSource` choice | **P** (missing: payment gateways, per-plan billing preferences, renewals) |
| Async tasks + artifacts | full | none | **M** |
| JS plugin runtime + marketplace | full (sobek) | none | **M** |
| Image / audio endpoints | ✓ | pass-through | P |
| Video generation | 11 providers | none | **M** |
| Midjourney | full action set | none | **M** |
| Realtime / Responses WS | ✓ | none | **M** |
| Batches / Files / Fine-tunes | ✓ | none | **M** |
| Moderations / Rerank / Edits | ✓ | rerank only (pass-through) | P |
| Vendors + Model metadata sync | ✓ (upstream sync) | vendors CRUD + metadata CRUD, no upstream sync | P |
| Model deployments (GPU rental) | ✓ | none | **M** |
| Perf metrics + rankings | ✓ | none | **M** |
| Check-in rewards | ✓ | none | **M** |
| Prefill groups / group ratio | ✓ | none | **M** |
| Multi-instance / system tasks | ✓ | none | **M** |
| Cache (Redis / disk) | ✓ | none | **M** |

---

## 6. Target: Superset Definition (measurable)

A superset claim is only meaningfully testable if it is expressed as counts and behaviors.

### 6.1 Parity gates (must all hold)

| Gate | Criterion |
|---|---|
| G1 Pages | Every unique NewAPI client route has an OxygenRouter equivalent rendering real data (no stubs) |
| G2 Settings | All 40 admin sections exist and persist to real storage |
| G3 Routes | All 328 NewAPI backend routes have an equivalent (same path or documented better path) |
| G4 Relay | All 50 relay endpoints implemented and protocol-correct |
| G5 Adapters | ≥40 wired adapters, each with a live connectivity test |
| G6 Billing | quota math matches NewAPI within **±0** for OpenAI-family token counts; pre-consume/settle/refund verified by a failure-injection test |
| G7 Auth | sessions, access tokens, passkey, TOTP 2FA, email verification, RBAC catalog all enforced |
| G8 i18n | ≥7 locales at ≥6,778 keys (or a documented, better key architecture with equal coverage) |
| G9 Tasks | async task lifecycle + artifacts + at least video providers |
| G10 Plugins | JS plugin runtime + marketplace install/activate/dryrun |

### 6.2 Superset deltas (must additionally hold — this is the "更强" part)

| # | OxygenRouter must beat NewAPI on | Mechanism |
|---|---|---|
| D1 | Single binary, zero external DB required for full function | keep SQLite-only default; optional PG/MySQL/Redis |
| D2 | Typed converter dispatch | extend `RelayValue` enum; no runtime `any` |
| D3 | Retry backoff with jitter (NewAPI: none) | exponential + jitter, configurable |
| D4 | Enforced global concurrency ceiling (NewAPI: not enforced) | tokio semaphore bound to `max_concurrent_requests` |
| D5 | Dynamic `/v1/models` from DB + adapter model lists | replaces static list |
| D6 | Full API-key update semantics | PUT update, not create/delete only |
| D7 | Design-system conformance (Apple design standards) as a gate | anti-slop checklist enforced in review |
| D8 | Real, reproducible benchmark harness proving parity | see §7 |

### 6.3 Rational re-arrangement (IA)

NewAPI's IA grew organically: 40 flat-ish settings sections, `/system-settings/{group}/{section}`
with a drill-in sidebar. We adopt the **drill-in workspace** pattern (it is correct) but
re-arrange for our product:

```
Console (root nav)
├── Overview
├── Analytics            ← overview | models | flow | users   (was: 2 pages)
├── Channels             ← channels | affinity | health
├── Tokens (keys)        ← tokens | limits | usage
├── Models               ← registry | vendors | pricing | deployments
├── Routing              ← rules | affinity | retry | rate-limit
├── Logs                 ← common | task | drawing | audit
├── Playground           ← chat | presets | chat2link
├── Wallet               ← balance | topup | orders
├── Usage & Billing      ← my usage | subscriptions | plans
└── Admin ───────────────────────────────────────────────┐
    ├── Users            ← users | sessions | 2FA | passkey
    ├── Billing          ← quota | currency | pricing | groups | payment | checkin
    ├── Models           ← global | per-vendor | deployment
    ├── Request policies ← checks | sessions&retry | health
    ├── Security         ← rate-limit | SSRF | token-limits
    ├── Content          ← announcements | api-info | FAQ | uptime | chat | drawing
    ├── Operations       ← behavior | alerts | email | worker | logs | perf | update
    └── System           ← info | notice | branding | nav | instances | tasks | plugins

Public
├── Home | Pricing | Rankings | Docs | About | Legal
└── Auth: sign-in | sign-up | otp | forgot | reset | oauth/* | setup wizard
```

Improvements over NewAPI's arrangement:

1. **Routing** is a first-class root workspace (NewAPI buries retry/routing in admin settings).
2. **Analytics** merges the four dashboard sections into one workspace with sections (fewer top-level items, same depth).
3. **Tokens** absorbs limits/usage (NewAPI splits `/keys` and token limits settings).
4. **Public Pricing/Rankings** promoted to first-class nav (as NewAPI does) *and* the same model-pricing data feeds the admin Models workspace — one source of truth.

---

## 7. Verification Protocol (real comparison, not assertions)

The user requires **real usage comparison and research**. This section defines the harness that
makes "superset" falsifiable.

### 7.1 Side-by-side differential harness

Run both engines against the **same upstreams** and diff behavior.

```
                       ┌──────────────────────────┐
   request fixtures ──►│  Differential Runner     │
   (frozen corpus)     │  (script, both engines)  │
                       └───────┬──────────┬───────┘
                               │          │
                    NewAPI :3000      OxygenRouter :3001
                               │          │
                               ▼          ▼
                    normalize(status, body, usage, tokens, channel picked)
                               │          │
                               └────┬─────┘
                                    ▼
                          diff report + verdict
```

Method:

1. **Corpus**: frozen request fixtures covering every relay endpoint (chat streaming/non-streaming,
   tools, vision, embeddings, images, audio, rerank, moderations, messages, responses, files, tasks).
2. **Same upstreams**: both engines configured with the same channel base URLs + keys (S3 already
   holds 11 real channels; reuse where licenses permit).
3. **Normalize**: strip volatile fields (ids, timestamps, latency), canonicalise `usage`.
4. **Assert**: for parity gates — status class, body schema, token counts (±0 for OpenAI family),
   channel-selection distribution within tolerance; for superset deltas — the OR-only behavior must
   be observable and absent in NewAPI.
5. **Report**: per-endpoint PASS/FAIL table, regenerated each gate.

### 7.2 Live instance reuse

The running NewAPI at `:3000` (v1.0.0-rc.37) is the reference oracle. It is reachable and
`/api/status` responds 200 with a full console config payload (`HeaderNavModules`,
`SidebarModulesAdmin`, `announcements`, `chats`, …). Its `/api/status` payload is itself a
**specification artifact** for the dynamic-IA feature and must be snapshotted into the harness.

### 7.3 UI comparison

1. Capture the NewAPI console with a browser driver (existing `.devlogs/newapi33_explore/*` and
   `.devlogs/napi_local/*` already hold 60+ screenshots of the reference UI).
2. Draft OxygenRouter redesigns in **Figma first** (MCP `figma` server: `https://mcp.figma.com/mcp`,
   plugin v2.0.20 enabled) — per project instructions, all GUI must be drafted in Figma and
   reference comparable products before implementation.
3. Implement, then diff against the Figma frame + NewAPI screenshot with the parity-check hook
   (`plugins/.../figma/scripts/post_write_figma_parity_check.sh`).

### 7.4 Anti-plagiarism gate

Because NewAPI is AGPL-3.0 and we are MIT:

- NewAPI source may be **read** to derive behavior, route names, and wire formats (interfaces are
  not creative expression).
- NewAPI **code must not be copied or transliterated**. Every implementation PR must state its
  derivation: "from observed behavior / wire format", never "from <file>:<line>".
- An automated check greps diffs for copied identifiers from AGPL sources where practical; the
  human gate is the AGPL awareness in this section.
- Where behavior is non-obvious (e.g. quota rounding, token de-holing), we derive it from
  **observations against the live oracle**, and record the observation as the evidence.

<!-- Gi tH ub @O xyg en AI Lab | Oxy gen AI La b@ Star sa ils Clov er -->

---

## 8. Execution Plan (dependency-ordered)

Ordering rule: wire what exists → build the two engines → then breadth.

| Phase | Scope | Why this order | Gate |
|---|---|---|---|
| **P0** | Repository hygiene + baseline: commit the restored tree, remove the 223 MB `ho/`+`ho.zip` redundancy, fix docs that mis-state the target, add `docs/research/` to the tracked set | The tree was found corrupted; do not build on that | `git status` clean, build/test green |
| **P1** | Wire `oxygenrouter-relay` into `proxy.rs`: registry, per-provider auth headers, multi-key selection, dynamic `/v1/models` | Unlocks 9 adapters of already-tested code; prerequisite for everything protocol-level | 9 adapters reachable via live channel test |
| **P2** | `oxygenrouter-billing`: ratio tables, tokenizer, quota math, atomic reserve, `BillingSession` pre/settle/refund | Without this the gateway is not a gateway; every later feature bills | Differential token/quota parity vs oracle |
| **P3** | Routing & reliability: priority tiers, weighted LB, affinity, auto-disable, retry+backoff, rate limiting, SSRF, concurrency | Makes multi-channel production-grade | Chaos/failover tests |
| **P4** | Protocol superset: moderations, batches, files, fine-tunes, count_tokens, native `/v1/messages`, `/v1beta`, Responses/Realtime WS | Completes G3/G4 | All 50 relay endpoints in the differential harness |
| **P5** | Auth & identity: sessions, access tokens, passkey, TOTP, email, OAuth×6, RBAC | Independent; needed by P6 | Live auth E2E per provider |
| **P6** | Payments & subscriptions: providers, webhooks, compliance, redemption, checkin | Needs P2+P5 | Sandbox webhook tests |
| **P7** | Async tasks, artifacts, video providers, Midjourney, JS plugin runtime + marketplace | Needs P2+P3 | Plugin install/activate/dryrun E2E |
| **P8** | Console: implement the re-arranged IA (all pages + 40 settings sections), Figma-first, 7 locales | Last, because it renders everything above | G1/G2 + UI parity report |
| **P9** | Perf/rankings/vendors/deployments/multi-instance/cache + final superset verification | Polish + proof | Full differential report with zero open parity failures |

### Immediate next step

**P0 then P1.** P0 is small and unblocks safe work; P1 is the single highest-leverage change in the
program because it converts 9 tested adapters and 101 public items from dead code into the product's
core. Both are verifiable within this session.

---

## 9. Open Decisions (owner input requested)

| # | Decision | Options | Recommendation |
|---|---|---|---|
| Q1 | Is the goal "superset of NewAPI as-shipped" or "superset of NewAPI + our own product direction"? | (a) strict parity (b) parity + deltas | **(b)** — §6.2 already lists the deltas |
| Q2 | Scope of first delivery: engine-first (P1–P4) or UI-first? | (a) engines (b) UI | **(a)** — UI without engines renders nothing real |
| Q3 | Multi-DB / Redis support? | (a) SQLite only (b) add PG/MySQL (c) add Redis too | **(a)+optional** — keep single-binary advantage (D1) |
| Q4 | Reuse of NewAPI AGPL code | (a) never (b) case-by-case with legal review | **(a)** — cleanest for MIT + commercial |
| Q5 | Deployment targets for parity tests | (a) local only (b) add the live oracle `:3000` | **(b)** — required for real comparison |
| Q6 | Locale set for G8 | (a) 2 (b) 7 | **(b)** — but after engines; wire the i18n architecture early |

---

*End of analysis. Regenerate this document whenever a gate passes.*

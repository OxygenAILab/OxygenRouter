# Page parity: NewAPI's console vs this instance

> GitHub@OxygenAILab | OxygenAILab@StarsailsClover

The goal this document serves is "page-by-page parity with NewAPI, verified live".
It is the checklist that decides when that is true, so every row names both the
reference page and the evidence for this instance's state.

## Method

| List | Source |
|---|---|
| Reference client routes | `web/src/routes/**` of `QuantumNous/new-api` @ `c2b7a9a` (TanStack Router file routes; 64 route files, 5 of them error pages) |
| Reference settings sections | `web/src/features/system-settings/*/section-registry.tsx` — **40 sections** in 8 groups |
| Our console routes | `crates/oxygenrouter-webui/web/src/App.tsx` `path="/ui/..."` — **37 routes** |
| Our API surface | `crates/oxygenrouter-webui/src/api.rs` route table + `Access` classification |

Status vocabulary:

* **live** — driven against a running instance in this repository's fixture
  harness (`.devlogs/live_task/`) or rendered in the browser during review.
* **route** — the page or endpoint exists in source; not driven end to end this
  session.
* **missing** — no counterpart. This is the work list.

## 1. Public (signed-out) pages

| Reference route | Reference page | Ours | Status |
|---|---|---|---|
| `/` | landing / home content | `/ui/overview` (same page signed in or out) | live |
| `/pricing`, `/pricing/:modelId` | public price catalogue and per-model detail | `/ui/pricing` over `GET /api/pricing` (`pricing_public.rs`), and `/pricing` redirects to it. Per-model detail is a dialog rather than its own route | live |
| `/rankings` | public model rankings | `/ui/rankings` over `GET /api/rankings` (`rankings.rs`) | live |
| `/about` | about copy | `/ui/about` over `GET /api/about`, same option shape as the reference | live |
| `/privacy-policy`, `/user-agreement` | legal copy | `/ui/privacy`, `/ui/terms` over `/api/privacy-policy`, `/api/user-agreement` | live |
| `/setup` | first-run setup wizard | bootstrap admin is created on first start (`ensure_bootstrap_admin`) | different by design |
| `(auth)/sign-in`, `/sign-up` | sign in / register | `/ui/sign-in`, `/ui/sign-up` | route |
| `(auth)/forgot-password`, `/reset`, `/user/reset` | password reset by email | — | **missing** |
| `(auth)/otp` | one-time-code step | sign-in accepts a 2FA code in the same form | different by design |
| `(auth)/oauth`, `/oauth/:provider` | six OAuth providers | — | **missing** |
| `(errors)/401,403,404,500,503`, `_authenticated/errors/$error` | error pages | `/ui/401`, `/ui/403`, `/ui/404`, `/ui/500`, `/ui/503`; unknown `/ui/*` paths render the 404 view | live |

## 2. Signed-in pages

| Reference route | Ours | Status | Notes |
|---|---|---|---|
| `/dashboard`, `/dashboard/$section` | `/ui/overview`, `/ui/analytics`, `/ui/perf-metrics` | live | uptime panel, pricing page and the performance view were driven in the browser on 2026-10-19 |
| `/channels` | `/ui/channels` | route | channel CRUD, key rotation, model fetch |
| `/keys` | `/ui/keys` | route | create / edit / reveal |
| `/models`, `/models/$section` | `/ui/models`, `/ui/model-registry`, `/ui/model-pricing` | live | pricing page driven live; registry route exists |
| `/playground` | `/ui/playground` | route | |
| `/usage-logs`, `/usage-logs/audit` | `/ui/logs`, `/ui/audit` | live | the audit view was driven in the browser on 2026-10-19, including the stored body of a channel create (credentials redacted) |
| `/wallet` | `/ui/wallet` | route | |
| `/subscriptions` | `/ui/subscriptions`, `/ui/plans` | route | |
| `/redemption-codes` | admin CRUD inside `/ui/admin/billing`, and the redeem form on `/ui/wallet` | live | a code created in the browser on 2026-10-19; the admin gate was checking `role === "admin"` and so refused the instance's own root account — see `isAdmin` |
| `/profile` | — (fields on `/ui/settings`) | partial | |
| `/security` | `/ui/security` over `/api/user/{2fa,sessions,token}` | live | driven live on 2026-10-19: sessions revoked, token generated/rotated/revoked, two-factor enabled with an independently generated TOTP code and then disabled |
| `/users` | `/ui/admin/users` | route | |
| `/task-plugins` | `/ui/task-plugins` over `/api/plugin/task*` | live | driven in the browser on 2026-10-19: a build uploaded, enabled and activated, and a driver hook dry-run from the console (which is what found the `call_hook` dispatch defect) |
| `/system-info` | `/ui/system` | route | |
| `/chat`, `/chat2link` | — | **missing** | reference's own chat UI, not a gateway feature |

## 3. Settings workspace

The reference splits its admin settings into 8 groups and 40 sections; this
instance serves one option catalogue (`OPTION_SCHEMA`, 50 keys) behind
`/ui/system-settings`. Functionally the same settings are reachable, but the
workspace is not a superset section-by-section:

| Group | Sections | Ours |
|---|---|---|
| site | 4 | partly (`SiteName`, `Notice`, `Footer`, `ServerAddress`) |
| auth | 5 | partly (registration, password policy, session TTL) |
| billing | 6 | partly (`QuotaPerUnit`, pre-consume, trust quota, currency) |
| models | 5 | partly (pricing tables, model sync interval) |
| operations | 7 | partly (log retention, data export, check-in) |
| security | 3 | partly (fetch/SSRF policy, audit log, login attempts) |
| request-policies | 3 | **missing** (the reference's request-policy editor) |
| content | 7 | partly: Uptime Kuma, announcements and the FAQ all have options and a public read (`/api/site`); the API-address block and chat/drawing presets are still missing |

## 4. Work list, in the order that unblocks the most

1. ~~**Rankings page**~~ — done 2026-10-19 (`f4ecc8d`).
2. ~~**Public pricing page**~~ — done 2026-10-19 (`bc49edb`): `GET /api/pricing`
   and `/ui/pricing`.
3. ~~**Security page**~~ — done 2026-10-19: `/ui/security` over the existing
   2FA, session and access-token endpoints.
4. ~~**Task plugins page**~~ — done 2026-10-19: `/ui/task-plugins` over the
   existing plugin endpoints, plus the `call_hook` dispatch fix that made a
   dry run able to call a driver hook at all.
5. **Redemption codes page** — redeem endpoint exists; admin CRUD exists at
   `/api/admin/redemption-codes`. **Covered**: the admin CRUD lives in
   `/ui/admin/billing` and the redeem form on `/ui/wallet`; the remaining work
   here was the root gate, now fixed.
6. ~~**Audit log view**~~ — done 2026-10-19: `/ui/audit`, plus recording a denied
   privileged write with the caller's identity, which the middleware dropped.
7. **Content sections** — announcements and the FAQ are done (2026-10-19,
   `/api/site`, with the overview rendering both); API info addresses and the
   chat/drawing presets are still open.
8. ~~**Perf metrics**~~ — done 2026-10-19: `perf_metrics` storage, recording on
   the relay path, `/api/perf-metrics[/summary]` and `/ui/perf-metrics`. Time to
   first token stays unmeasured (streaming is buffered), so that field is
   omitted rather than reported as a measured zero.
9. **Ratio sync** (`/api/ratio_sync`) — **partial**. The root-only
   `/ui/ratio-sync` page and `/api/ratio_sync/{channels,fetch}` endpoints were
   driven live on 2026-10-19: operator-supplied HTTP sources and enabled
   channels are compared across `model_ratio`, `completion_ratio`, `cache_ratio`,
   `create_cache_ratio` and `image_ratio`; only real differences are reported,
   source-only models are separated, a dead source does not discard a live one,
   and the preview writes nothing (saving remains in the versioned pricing
   editor). The reference's official preset, models.dev and OpenRouter adapters,
   its remaining price fields (`model_price`, audio and billing-expression
   fields), and its richer response/selection shape are still open.
10. ~~**System tasks**~~ — done 2026-10-19: the five `/api/system-task/*` routes
    over a `log_cleanup` runner with durable progress, plus `/ui/system-tasks`.
11. ~~**Legal / about and error views**~~ — done 2026-10-20: the three option
    keys, the three reference paths and three console pages, linked from the
    overview footer; `/ui/401`, `/ui/403`, `/ui/404`, `/ui/500` and `/ui/503`
    are each drawn with recovery actions, and an unknown `/ui/*` path lands on
    the 404 view instead of silently rendering the overview. All five status
    views were driven in the browser on 2026-10-20.
12. **OAuth + password reset** — the only auth surfaces still missing.

## 5. What "superset" will mean here

A page counts as done when, in addition to existing, it has been driven against
a live instance and its effect observed:

* a page load with real data (browser, screenshot in the session log), and
* a probe in `.devlogs/live_task/` that asserts the behaviour the page exists
  for, or
* an existing probe that covers it, named in this table.

Rows marked **missing** are the honest remainder; the objective is not met while
any of them remains.

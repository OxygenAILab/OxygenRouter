# Changelog

All notable changes to OxygenRouter are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Fixed (security — log and analytics scoping) — **high**
- **Any signed-in user could read the instance's entire request-log table**, plus
  `/api/log/stats`, `/api/dashboard` and `/api/analytics/flow`. Verified live
  before the fix: an ordinary user's `GET /api/logs` returned other users' rows.
  The reference gates its log listing on `AdminAuth` and serves a separate
  `/log/self` for a user's own rows (`router/api-router.go:314,319`).
- `/api/logs`, `/api/log`, `/api/logs/stream`, `/api/dashboard` and
  `/api/analytics` joined the admin class, and the existing
  `GET /api/logs/:id/secret`-style exception mechanism gained `/api/logs/self`, so
  a user can still see their own activity.
- Scoping is resolved in SQL (`query_request_logs_scoped`), not by filtering the
  page afterwards: a post-filter would report a `total` and page boundaries the
  caller cannot actually see. It also means `?api_key_id=someone-else` cannot
  widen the scope, because the owner filter is applied independently of the
  caller's query parameters. The console uses `/api/logs/self` for non-admins.

### Verified (security — log and analytics scoping)
- Live with two ordinary users each making a request: both now get `403` on
  `/api/logs`, `/api/log/stats`, `/api/dashboard` and `/api/analytics/flow`;
  each sees exactly **one** row through `/api/logs/self` (their own, not the
  other's); and the admin still sees both rows through `/api/logs`.
- Two unit tests pin the class assignment, including that `/api/logs/self` stays
  user-reachable while `/api/logs` and `/api/logs/anything-else` stay admin-only.
- `cargo test --workspace` → **434 passed / 0 failed**, zero warnings. `tsc
  --noEmit` is clean, which is what caught the frontend's `User` type still
  claiming `role` was only `"admin" | "user"` after the root role landed.

### Fixed (security — API keys and secret options) — **high**
- **`GET /api/keys` and `GET /api/keys/query` returned every key's real value.**
  The reference masks in *both* (`controller/token.go:140,157`, via
  `model.MaskTokenKey`), and the console's own display masking was cosmetic — the
  raw value was still in the response body, so a browser cache, a proxy log or a
  screenshot held a working credential. Both routes now return the reference's
  mask (`abcd**********wxyz`, `ab****yz`, `****`), ported branch for branch
  including its short-key cases. Verified live before the fix that the raw key was
  present.
- Added `GET /api/keys/:id/secret` as the one deliberate disclosure route,
  mirroring the reference's separate credential route. It is owner-or-admin
  gated, so a masked list plus this route replaces an unmasked list, and the
  disclosure is explicit, separately logged, and never a side effect of a bulk
  read. The console's copy buttons now use it.
- **A secret option could be overwritten by its own preview.**
  `PUT /api/options` wrote the value blindly, so a client that read the masked
  list and saved it back would replace the real secret with the mask — the same
  round-trip footgun that destroyed channel credentials. A value ending in the
  preview ellipsis is now treated as unchanged, and a genuine value still saves.

### Verified (security — API keys and secret options)
- Live: a created key is returned in full once (creation is when the caller needs
  it); the list and the search both mask it with the reference's format and hide
  the interior; `GET /api/keys/:id/secret` returns it to the owner, refuses
  another user ("not your key"), and refuses an anonymous caller (`401`).
- Live: on a stored secret, saving the preview that `GET /api/options` returned is
  reported as `unchanged` and leaves the real value intact, while setting a
  genuinely new value still applies.
- Five unit tests cover the masking algorithm (all four length branches), that a
  realistic key's interior is hidden, that multi-byte input cannot panic a byte
  slice, that other fields survive masking, and that a preview is recognised as a
  no-change signal.
- `cargo test --workspace` → **432 passed / 0 failed**, zero warnings. `npm run
  build` succeeds; `tsc --noEmit` is clean.

### Fixed (security — channel credentials) — **high**
- **`GET /api/channels` returned each channel's raw upstream API key**, and so did
  `GET /api/channels/:id` and the response to `POST /api/channels/:id/keys`. The
  reference omits the key from its channel list entirely
  (`controller/channel.go:251`, `.Omit("key")`). Verified live before the fix.
  Read and update responses now carry a placeholder (`••••••••`), so a client can
  still tell "configured" from "not set" without being able to read the value.
- **Saving a channel destroyed its credential.** The console reads the list,
  edits one field, and PUTs the whole object back; with a masked or blank `api_key`
  that overwrote the real one. Verified live: a round-trip PUT left the channel
  with `api_key = ""`, silently disabling it. `update_channel` now treats the
  placeholder *or* a blank as "leave unchanged", and preserves `created_at`.
  Rotating to a genuinely new key still works, as does the relay — both asserted.
- **`key_preview` disclosed short keys entirely.** It returned the full value
  whenever the key was ten characters or fewer — a full disclosure for exactly the
  short, low-entropy credentials most worth protecting. Now: a four-character
  prefix for long keys, and nothing but an ellipsis for short ones.
- `manage_channel_keys` no longer returns the credentials it was given back to
  the caller.

### Verified (security — channel credentials)
- Live: a channel configured with two keys (one long, one short) discloses neither
  through the list, the single-channel read, or the key-status previews; a
  round-trip save with the masked value keeps both keys and still applies the real
  edit; a blank-key save also preserves them; an explicit new key rotates
  successfully; and the upstream test still passes with a usable credential.
- Seven unit tests cover masking (value hidden, other fields intact, empty stays
  empty), both "unchanged" signal shapes, and that a short key is never shown in
  full.
- `cargo test --workspace` → **427 passed / 0 failed**, zero warnings.

### Fixed (security — console authentication) — **critical**
- **49 of 86 console routes were reachable without any credential.** Each handler
  authenticated itself, and a handler that forgot to call `auth_user` was simply
  open, with nothing marking it as such. Verified anonymously against a live
  instance before the fix:
  - `GET /api/channels` returned every channel **including its upstream API key**.
  - `PUT /api/channels/:id` repointed a channel's `base_url` — silently
    redirecting all traffic, and the credential, to an attacker's host.
  - `POST /api/keys` created a working API key attributed to nobody.
  - `GET /api/settings` returned `local_api_token`; `GET /api/options` returned
    all instance configuration; `GET /api/logs`, `/api/dashboard`,
    `/api/models-metadata`, `/api/vendors`, `/api/model-maps` and `/api/rules`
    were open as well.
- Replaced per-handler checks with **one deny-by-default path-class gate** over
  the whole `/api` tree (`access_guard` + `required_access` in `api.rs`). An
  unclassified route now requires a session instead of being open, so the failure
  mode of a new route is closed rather than exposed. Classes mirror the
  reference's guards: public allow-list, any user, admin, and root-only for the
  instance-wide surfaces (`option`, `settings`, `system-info`, `backup`, `plugin`,
  `system-task`), matching its `RootAuth` groups.
- Added a distinct **root** role, because the reference separates the instance
  owner from an admin (its bootstrap user is role 100, root) and gates option
  writes and system tasks on root alone. The bootstrap account is now root, which
  also avoids locking the owner out of surfaces only root may reach.
- **API keys are now scoped to their owner.** `list_keys` and `keys_usage` filter
  to the caller (admins see all), `delete_key` refuses a key belonging to someone
  else, and `create_key` takes ownership from the credential rather than the
  request body — trusting a body-supplied `user_id` would have let any user create
  keys billed to another's wallet.

### Verified (security — console authentication)
- The four attacks above re-run anonymously against a live instance now return
  `401`, while `/api/status` and `/api/plans` stay public and the owner still
  reaches the root-only surfaces (`200`). An ordinary user gets `403` on
  `/api/channels`, `/api/settings`, `/api/options` and `/api/admin/users`, and
  `200` on `/api/keys`.
- Cross-user isolation: alice's key is invisible to bob, bob's delete attempt is
  refused ("not your key"), and alice's key survives. A user asking for a key
  attributed to someone else gets it attributed to themselves.
- Path classification is unit-tested (8 cases), including that a lookalike prefix
  like `/api/optionsfoo` does not inherit the `/api/options` class, that an
  unclassified route fails closed, and that an unrecognised role string does not
  escalate.
- `cargo test --workspace` → **420 passed / 0 failed**, zero warnings.

### Added (P6 — subscription quota funds requests)
- **A subscription's pool now actually pays for requests.** `FundingSource`
  (`Wallet` or `Subscription { user_id, subscription_id }`) chooses who funds a
  request, mirroring the reference's abstraction
  (`service/funding_source.go`), and the proxy selects a subscription when one can
  cover the reservation. A `Subscription` source spends its pool first and the
  owner's wallet for anything beyond it.
- `reserve_subscription_quota` / `adjust_subscription_quota` /
  `restore_subscription_quota` on the store, plus the matching `BillingStore`
  methods and the `DualStore` split that routes a subscription account between
  pool and wallet.

### Verified (P6 — subscription quota funds requests)
- **Decisive live proof.** A user with a **zero wallet** is refused with `403`
  (`insufficient balance: need 99, available 0`); granting them a plan makes the
  identical request return `200` with content, and the pool records `amount_used
  = 27` while the wallet stays at `0`. Invalidating the plan returns the request
  to `403`. That is the whole claim, end to end.
- Partial reservation: a pool smaller than the charge is drained and the wallet
  covers the remainder, rather than the pool being skipped while its remaining
  quota is stranded until expiry.
- A refused reservation rolls the pool back, so a request that cannot be funded
  costs nothing — asserted at the store level and exercised by the live `403`.
- Settlement beyond the pool becomes wallet debt rather than a negative
  subscription balance, matching the wallet's existing overrun rule.
- An unlimited pool (`amount_total = 0`) funds any amount and records no usage.
- The pre-existing wallet-only path is asserted unchanged, so no existing
  deployment's billing shifts.
- **A bug the tests caught, twice.** `restore_subscription_quota` originally
  bounded the refund by the pool's *remaining room* instead of by what had been
  used, so a drained pool — the case that matters — restored nothing and an
  untouched pool would have gone negative. The same mistake was in the billing
  test double; both are fixed, and the reason is in the code comment.
- `cargo test --workspace` → **412 passed / 0 failed**, zero warnings.

### Added (P6 — subscription quota pool)
- **A subscription's quota is now a real, spendable pool.** `subscriptions` gains
  `amount_total` (snapshotted from the plan's `quota_micros` at creation) and
  `amount_used`, plus `subscription_funding_source`, `consume_subscription_quota`
  and `refund_subscription_quota`.
- Selection follows the reference (`model/subscription.go:1334-1358`): among
  active, unexpired subscriptions ordered by soonest expiry then id, the first
  whose pool can still cover the amount. Draining the nearest-to-lapsing
  entitlement first is what stops a user losing paid-for quota when one expires.

### Verified (P6 — subscription quota pool)
- **The pool lives on the subscription, not the wallet**, matching the reference
  (`AmountTotal`/`AmountUsed`). This is the mechanism I had earlier recorded
  incorrectly in the roadmap as "the reference credits the user"; it does not, and
  the roadmap is corrected. The distinction is load-bearing: crediting the wallet
  would make subscription quota spendable *after* expiry and would blend it with
  money the user can top up. A test asserts an expired subscription with quota
  left funds nothing.
- Editing a plan does not change an existing subscription's pool, because the
  total is a snapshot — asserted, since a later reprice must not retroactively
  alter what was bought.
- Spending is guarded in the `WHERE` clause, so a concurrent request cannot drive
  usage past the total — the same atomic-reserve pattern as the wallet. An
  exhausted pool refuses further spend; a refund is clamped at zero so a double
  refund cannot manufacture quota.
- **A bug the tests caught:** the first `consume` implementation required
  `amount_total > 0`, so an *unlimited* pool (`0`) rejected every spend. The
  reference accepts those and records no usage. Fixed with a single statement that
  expresses both cases, so there is one definition of "may this spend proceed".
- `cargo test --workspace` → **405 passed / 0 failed**, zero warnings.

### Added (P6 — subscription admin lifecycle)
- Seven routes under `/api/subscription/admin/*`, matching the reference's
  `subscriptionAdminRoute` group: `bind`, a user's subscriptions, a plan's
  subscribers, a user reset, a plan reset, `invalidate`, and `delete`. Previously
  an admin could create plans but could not see or change who held them.

### Fixed (P6 — subscription regression)
- **Buying a plan cancelled the user's other plans.** Our `subscribe` ended every
  other active subscription, so a user could hold only one. That is *less* than
  NewAPI, which allows concurrent subscriptions and bounds repeats per plan
  (`model/subscription.go`: `MaxPurchasePerUser`); the behaviour silently
  destroyed an entitlement the user had paid for. A new test that granted two
  plans and expected both to survive caught it — an existing unit test had
  encoded the old behaviour as correct ("replaces prior subscription"), which is
  why it had gone unnoticed. Both tests were corrected and the reasoning is now in
  the function's doc comment.

### Verified (P6 — subscription admin lifecycle)
- **Granting does not charge**, matching `AdminBindSubscription`: a test asserts
  the wallet and ledger are unchanged, alongside a contrasting test that a
  *purchase* does debit, so the difference is real rather than an artefact of an
  empty wallet.
- **Invalidate ≠ delete**: invalidating keeps the row so the audit trail survives
  and reports `404` on an already-inactive row rather than rewriting history;
  deleting erases a mistaken grant.
- Grants accept a disabled plan, and the plan-wide reset walks the per-user
  primitive so there is one definition of "end a subscription".
- Live: two plans granted concurrently (both stay active, balance unchanged), a
  duplicate grant refused with `409`, subscriber listing, invalidate leaving the
  other plan running, a repeat invalidate `404`, a plan reset, a delete, and a
  customer token refused with `403`. Sixteen storage tests cover the same ground.
- `cargo test --workspace` → **389 passed / 0 failed**, zero warnings.

### Added (P5 — permission catalog)
- **`GET /api/authz/catalog`**, admin-gated like the reference
  (`router/authz-router.go:14-17`): the resource/action registry plus each role's
  baseline grant matrix, in `crates/oxygenrouter-core/src/authz.rs`. This is the
  schema a permission editor renders.
- Registry contents match the reference exactly (`service/authz/resources_*.go`):
  `channel` × {`read`, `operate`, `write`, `sensitive_write`, `secret_view`},
  `audit` × `read`, `task_plugin` × `bind`.

### Verified (P5 — permission catalog)
- The channel actions are five distinct privileges, and the **admin baseline
  deliberately excludes** `sensitive_write`, `secret_view`, `audit/read` and
  `task_plugin/bind`, while `root` is a superuser holding everything. Both facts
  asserted live, since a catalog whose whole purpose is to separate privileges
  would be pointless if its own baseline collapsed them.
- Grants are **computed from the registry, never stored**, so the matrix cannot
  disagree with the action definitions. A test checks every cell of every matrix
  against the per-action lookup.
- `default_roles` is not serialized (the reference tags it `json:"-"`); the live
  check asserts an action object's keys are exactly
  `action`/`label_key`/`description_key`, so the two payloads stay interchangeable.
- A superuser is **not** a wildcard: an unregistered action reads as not-granted
  rather than passing because the role is `root`, so a typo in a future check
  cannot silently succeed.
- Live: anonymous `401`, a normal user `403`, admin receives the catalog, and the
  grant matrices match the expected baseline exactly.
- **Not enforcement.** This is the catalog, not a policy engine; the reference
  evaluates permissions through Casbin on every admin route and we do not yet.
  Recorded in the roadmap rather than implied to be at parity.
- `cargo test --workspace` → **373 passed / 0 failed**, zero warnings.

### Added (P5 — two-factor authentication)
- **TOTP 2FA wired to HTTP**: `/api/user/2fa/{status,setup,enable,disable,backup_codes}`,
  matching the reference's paths (`router/api-router.go:129-133`), plus login
  gating and recovery codes.

### Verified (P5 — two-factor authentication)
- **A correct password no longer yields a session when 2FA is on.** Login accepts
  an optional `code`; without a valid TOTP or recovery code no `session_token` is
  issued. A second factor that still handed out a session would be decorative, so
  the live check asserts the token is absent, not merely that the status is `401`.
- Verified against an **independent from-spec TOTP implementation**, not just our
  own: password-only login refused after activation, password + wrong code
  refused, password + an externally generated code accepted, a recovery code
  accepted then rejected on replay, and disable refused with a wrong code.
- **Setup stages; enable activates.** A mis-scanned or discarded QR code cannot
  lock an account out, because the secret only takes effect once a valid code
  proves the operator can generate one.
- **Recovery codes are stored as salted Argon2 digests**, never in the clear, and
  each is consumed on use. A test reads the raw database column and asserts the
  plaintext is absent.
- Disable requires the second factor (it is what an attacker with a stolen
  session would want), and clears the secret and codes so a later re-enable
  cannot silently reuse a revoked value.
- **A real bug the tests caught:** the first implementation compared backup codes
  by hashing the candidate and comparing digest strings. That cannot work —
  Argon2 embeds a random salt, so two hashes of the same code differ and every
  recovery code would have been rejected. Verification now parses the stored PHC
  string and calls `verify_password`.
- `cargo test --workspace` → **362 passed / 0 failed**, zero warnings.

### Added (P5 — TOTP primitive)
- **RFC 6238 TOTP** in `crates/oxygenrouter-core/src/totp.rs`, with the parameters
  the reference uses: HMAC-SHA1, 30-second period, six digits. Built against the
  RFC rather than dependency-pulled so those parameters are explicit, plus
  RFC 4648 base32 (no padding, tolerant of padding and lower case on input),
  CSPRNG-backed secret generation, and `otpauth://` provisioning URIs with
  percent-encoded labels.

### Verified (P5 — TOTP)
- **Against the RFC's own published vectors.** Appendix B prints eight-digit codes
  for a known seed; `tests/totp.rs` asserts the generator reproduces all six SHA1
  rows exactly. That is stronger than a self-consistent test, since a
  wrong-but-tidy implementation still disagrees with every authenticator app.
- Two corrections the vectors caught, both now pinned by tests:
  1. A six-digit code is the eight-digit value **modulo 10⁶** — the *low* six
     digits. Taking the leading six gives `942870` where the answer is `287082`.
     Confirmed against an independent from-spec implementation before changing
     anything, so the fix followed evidence rather than the failing assertion.
  2. `hotp_value` must return the raw 31-bit truncation *before* `mod 10^digits`;
     folding the modulus in made the eight-digit assertion re-reduce an
     already-reduced value (`00287082`).
- The `base32` encoder is checked against the RFC 4648 `foobar` vector, not just a
  round trip. The ±1-step window is asserted to accept a neighbouring step and
  reject one two steps away. Comparison is constant-time; malformed input (empty,
  short, long, non-digit, full-width digits) is rejected without panicking.
- **Not yet wired to HTTP.** No `/api/user/2fa/*`, no login gating, no backup
  codes, no lockout. This lands the verifiable primitive; the API surface is next.
- `cargo test --workspace` → **350 passed / 0 failed**, zero warnings.

### Added (P5 — access tokens)
- **Access tokens** — `GET`/`POST /api/user/token` (generate),
  `DELETE /api/user/token` (revoke), `GET /api/user/token/status`, matching the
  reference (`router/api-router.go:102-105`). A token is now accepted anywhere a
  session token is, so a script can drive the user API without a browser session.

### Verified (P5 — access tokens)
- Shape is the reference's 29–32 alphanumeric characters, with entropy taken from
  UUIDv4 bytes (CSPRNG-backed) rather than a weaker source. A generated value is
  checked against existing tokens so a collision is retried rather than silently
  handing one user another's credential.
- Regeneration **rotates**: verified live that the previous value answered `401`
  while the new one answered `200`.
- **The value is returned exactly once.** `status` reports `enabled` and
  `created_at` only; the live check asserts the token string does not appear in
  the status response, because returning the stored row would pass every other
  assertion while exposing a live credential.
- Lookup joins on `status='active'`, so disabling an account ends its API access
  without a separate revocation step (test-asserted).
- Live end to end: status, generate (length 32), status with the value absent,
  authenticate `/api/auth/me` with the token, rotate, revoke, and an
  unauthenticated generate refused with `401`. Eight storage tests cover the same
  ground.
- `cargo test --workspace` → **330 passed / 0 failed**, zero warnings.

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

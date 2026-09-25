# Changelog

All notable changes to OxygenRouter are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added
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

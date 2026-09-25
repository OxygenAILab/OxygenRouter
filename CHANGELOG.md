# Changelog

All notable changes to OxygenRouter are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

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

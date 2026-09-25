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

> **Not yet wired:** `crates/oxygenrouter-relay` provides 9 provider adapters
> (OpenAI, Anthropic, Gemini, Bedrock+SigV4, Vertex, Ollama, Cohere, Azure,
> AdvancedCustom) plus OpenAI↔Claude / OpenAI↔Gemini converters and SSE state
> machines — 101 public items, 15 unit tests passing — but `oxygenrouter-webui`
> does not yet depend on it, so production traffic still uses the legacy
> pass-through client. This is the top priority of the superset program.

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
- `cargo test --workspace` → **20 passed / 0 failed** (15 in `relay`, 5 in `core`)
- `npm run build` (tsc + vite) → **passes** (2,032 modules)
- `cargo check --workspace` → clean

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

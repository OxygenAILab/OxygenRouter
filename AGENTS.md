# AGENTS.md — OxygenRouter development guide

> **GitHub@OxygenAILab | OxygenAILab@StarsailsClover**

This document is for AI agents and human contributors working on OxygenRouter. It captures the engineering discipline and conventions.

## 1. Engineering principles

1. **Evidence before claims** — every behavior claim must be backed by code at `file:line`. No "should work" assertions without a verified test path.
2. **Modular planning** — large work (>=3 modules or >=500 lines) gets a todo list with status updates in real time.
3. **Edit safety** — no silent assumptions. If a function expects `id` but we have a `name`, surface the mismatch, don't paper over it.
4. **Audit integrity** — no fake reviews, no "tested locally" claims without a real test invocation and recorded output.
5. **Version discipline** — follow `v{Year}.{Major}-Alpha {AlphaVer.}` (BC convention). Current line: `v26.0-Alpha 1`.
6. **Watermark** — every code/document file should carry `GitHub@OxygenAILab | OxygenAILab@StarsailsClover` in the top module comment.

## 2. Project layout

```
OxygenRouter/
├── Cargo.toml                       # workspace root
├── README.md
├── CHANGELOG.md
├── AGENTS.md
├── .gitignore
├── .devdocs/                        # design notes
├── .devlogs/                        # development session logs
├── docs/                            # long-form documentation
├── releases/                        # built binaries (gitignored)
├── src/
│   └── bin/oxygenrouter/main.rs     # CLI entry, Windows console subsystem
└── crates/
    ├── oxygenrouter-core/           # data models + SQLite
    ├── oxygenrouter-proxy/          # upstream client + scheduler + router
    └── oxygenrouter-webui/          # axum REST API + static webui
        └── web/                     # React 18 + Vite + Tailwind
```

## 3. Tech stack & versions

| Layer        | Choice                              | Why                                       |
|--------------|-------------------------------------|-------------------------------------------|
| Backend      | Rust 1.75+ (stable MSVC)            | Memory safety + single-binary deployment  |
| HTTP server  | axum 0.7                            | Tokio-native, ergonomic                   |
| HTTP client  | reqwest 0.12 (rustls)               | Streaming support                         |
| Database     | SQLite via rusqlite (bundled)       | Zero external dependency                  |
| Frontend     | React 18 + Vite 5 + Tailwind 3      | Match OxygenClaw stack                    |
| Icons        | lucide-react                        | Match OxygenClaw                          |
| State        | @tanstack/react-query               | Server-state management                   |

## 4. Data model (SQLite)

| Table         | Purpose                                            |
|---------------|----------------------------------------------------|
| channels      | Upstream API providers (base_url, api_key, prio, weight) |
| api_keys      | Local client tokens                                |
| model_maps    | Per-channel model rewriting (pattern → target)     |
| route_rules   | Reserved (priority/type-based rules)               |
| request_logs  | Audit trail: method, path, model, channel, status, duration |
| settings      | KV for app config (listen_host, port, local_token) |

## 5. Protocol surface

Implemented OpenAI-compatible endpoints (under `127.0.0.1:<port>/`):

| Method | Path                       | Notes                                     |
|--------|----------------------------|-------------------------------------------|
| POST   | /v1/chat/completions       | Streaming (SSE) + non-streaming           |
| POST   | /v1/completions            | Legacy completions                        |
| POST   | /v1/embeddings             | Pass-through                               |
| POST   | /v1/images/generations     | Pass-through                               |
| GET    | /v1/models                 | Static list of well-known model IDs       |
| GET    | /api/channels              | CRUD: channels                            |
| GET/POST/PUT/DELETE | /api/channels/:id    | Channel management                        |
| POST   | /api/channels/:id/test     | Connectivity test                         |
| GET/POST | /api/keys                | Local API keys                            |
| GET/POST | /api/model-maps          | Model rewriting                           |
| GET/POST | /api/rules               | Route rules (reserved)                    |
| GET    | /api/logs                  | Request log with `?limit=N`               |
| GET    | /api/status                | System status snapshot                    |
| GET/PUT | /api/settings             | App config                                |

## 6. Channel selection algorithm

```
pick_channel(exclude):
  enabled = SELECT * FROM channels WHERE enabled=1 AND id NOT IN (exclude)
  total_weight = SUM(weight) over enabled
  target = random() mod total_weight
  acc = 0
  for c in (sorted by priority DESC, weight DESC):
    acc += c.weight
    if acc > target: return c
```

Retry policy (default `max_retries=3`):
- 408, 425, 429, 500, 502, 503, 504, 401, 403 → switch channel
- network error → switch channel
- `context_length_exceeded` in body → try long-context fallback on same channel, then continue
- 400 (other) → return upstream error to client

## 7. Model routing heuristic (`model=auto`)

| Detected feature in request           | Resolved model              |
|---------------------------------------|-----------------------------|
| Path = /v1/images/*                   | `dall-e-3`                  |
| Path = /v1/audio/*                    | `tts-1`                     |
| Path = /v1/embeddings                 | `text-embedding-3-small`    |
| Message contains image_url            | `gpt-4o`                    |
| Total message chars > 16k             | `gpt-3.5-turbo-16k`         |
| tools/function field present          | `gpt-4o-mini`               |
| Default (plain chat)                  | `gpt-3.5-turbo`             |

Long-context fallback table (in `proxy::router`):
- `gpt-3.5-turbo` → `gpt-3.5-turbo-16k`
- `gpt-4` → `gpt-4-32k`
- `claude-3-haiku` → `claude-3-sonnet`
- `claude-3-sonnet` → `claude-3-opus`

## 8. Build & release

### Dev build

```bash
cargo build                       # debug
cargo build --release             # optimized (~2-5 MB binary)
```

### Frontend build (required for embedded WebUI)

```bash
cd crates/oxygenrouter-webui/web
npm install
npm run build
```

The `npm run build` outputs to `crates/oxygenrouter-webui/web/dist/`, which is then embedded via `include_dir!` at Rust compile time.

### Cross-platform targets

```bash
# Windows x86_64
cargo build --release --target x86_64-pc-windows-msvc

# Linux x86_64
rustup target add x86_64-unknown-linux-gnu
cargo build --release --target x86_64-unknown-linux-gnu

# macOS Apple Silicon
rustup target add aarch64-apple-darwin
cargo build --release --target aarch64-apple-darwin
```

## 9. Watermark rule

Every source file MUST start with a watermark in its module-level doc comment:

```rust
//! <file purpose>
//!
//! GitHub@OxygenAILab | OxygenAILab@StarsailsClover
```

For documentation files (`*.md`), put it on a comment line near the top.

## 10. Commit & PR conventions

- Use **Conventional Commits** (`feat:`, `fix:`, `chore:`, `docs:`, `refactor:`, `test:`)
- Subject ≤ 50 characters
- Body explains "why", not "what"
- One concern per commit

## 11. Testing checklist (before tagging an Alpha)

- [ ] `cargo build --release` succeeds
- [ ] `cargo test` passes (currently no tests; add when refactoring)
- [ ] `cd crates/oxygenrouter-webui/web && npm run build` succeeds
- [ ] Manual smoke: start binary, open WebUI, add a channel, run a chat, verify failover by toggling channels
- [ ] `git status` is clean (no stray build artifacts)
- [ ] `CHANGELOG.md` updated with this Alpha's notable changes

## 12. Known limitations (Alpha 1)

- No multi-user / quota / billing
- Only OpenAI-compatible upstream (no Anthropic / Gemini adapters yet)
- Local API key check is permissive — any `Bearer xxx` accepted (only channel credentials are validated upstream)
- No HTTPS for local server (rely on local trust)
- Model map uses simple glob patterns; no full regex
- Logs are in SQLite only; no streaming export

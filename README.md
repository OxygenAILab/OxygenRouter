# OxygenRouter

> **O₂** — OpenAI-compatible API gateway with channel rotation, model routing, and WebUI. Built in Rust.
>
> GitHub@OxygenAILab | OxygenAILab@StarsailsClover

OxygenRouter is a single-binary CLI / WebUI that sits between your OpenAI-compatible clients (Cursor, Continue, OpenAI Python SDK, etc.) and multiple upstream API providers. It provides:

- **Channel rotation** — multiple upstream APIs with priority/weight, automatic failover
- **Error-aware switching** — 401/403/429/5xx/Time-out errors trigger channel rotation; `context_length_exceeded` triggers long-context model fallback
- **Model routing** — `model=auto` heuristic, type-based routing (text/vision/image), keyword matching
- **WebUI** — Dashboard, channels, API keys, model maps, route rules, and request logs
- **Bilingual** — English and Chinese UI with auto-detection and one-click switching
- **Local auth token** — set your own OpenAI-protocol token; clients send it to `localhost` and OxygenRouter forwards to upstream

Inspired by [NewAPI](https://github.com/songquanpeng/one-api) and the WebUI style of [OxygenClaw](https://github.com/OxygenAILab/oxygen-claw).

---

## Quick start (distribution package)

### Windows (recommended)

1. Download the latest `oxygenrouter-x86_64-pc-windows-msvc.zip` from the [Releases page](https://github.com/OxygenAILab/OxygenRouter/releases).
2. Extract the archive anywhere (e.g. `C:\Apps\OxygenRouter`).
3. Double-click `oxygenrouter.exe`.

A console window opens, the WebUI is served on `http://127.0.0.1:3001`, and your default browser opens automatically to the local dashboard. No further setup is required.

### Build from source

Prerequisites: Rust 1.75+, Node 20+, npm

```bash
git clone https://github.com/OxygenAILab/OxygenRouter
cd OxygenRouter

# 1) Build the WebUI
cd crates/oxygenrouter-webui/web
npm install
npm run build
cd -

# 2) Build the Rust binary (embeds the WebUI)
cargo build --release
./target/release/oxygenrouter.exe   # Windows
./target/release/oxygenrouter       # Linux/macOS
```

### One-shot release build (Windows)

```bash
scripts\build_release.bat
```

This script installs npm dependencies, builds the WebUI, compiles the release binary, then packages everything into `release/oxygenrouter-x86_64-pc-windows-msvc.zip`.

### Use as an OpenAI client

```bash
curl http://localhost:3001/v1/chat/completions \
  -H "Authorization: Bearer <your-local-api-token-from-settings>" \
  -H "Content-Type: application/json" \
  -d '{
    "model": "auto",
    "messages": [{"role": "user", "content": "Hello, who are you?"}]
  }'
```

The local API token is shown on the Settings page (and on first start in the console banner). Change it any time; settings are persisted to `config.json` and `oxygenrouter.db` next to the binary.

---

## Architecture

```
crates/
├── oxygenrouter-core/   # data models, SQLite, config
├── oxygenrouter-proxy/  # upstream client, channel scheduler, model router
└── oxygenrouter-webui/  # axum REST API + embedded Vite/React static
    └── web/             # React 18 + Vite + Tailwind + lucide (OxygenOrigin dark)
src/bin/oxygenrouter/    # CLI entry — console subsystem on Windows
```

### How channel rotation works

1. Client sends OpenAI-format request → OxygenRouter at `localhost:3001/v1/chat/completions`
2. Router inspects `model` field:
   - If `auto` → heuristic: vision? long context? tools? → choose a real model
   - If explicit → look up Model Map for the chosen channel (rewrites `gpt-4` → `gpt-4-turbo` etc.)
3. Scheduler picks a channel: priority DESC, then weighted random among candidates
4. Send to upstream. On response:
   - 2xx → return to client
   - 401/403/429/5xx/network → **switch channel**, retry (up to `max_retries`)
   - `context_length_exceeded` → **try long-context fallback** (e.g. `gpt-3.5-turbo` → `gpt-3.5-turbo-16k`) on same channel, then continue
5. Every request is logged with channel, model, status, duration

### WebUI pages (English / 中文)

- **Overview** — welcome panel, base address, 4-step setup checklist (channel → key → mapping → first test), and a tile grid of local services
- **Model Analytics** — real charts: time-bucket request line + error overlay, per-model and per-channel horizontal bars, channel health cards
- **Channels** — card grid with status dot, search, show-disabled toggle, per-card test / copy / edit / delete
- **API Keys** — row cards with masked-by-default keys, reveal-once, copy, and enabled/disabled badge
- **Routes** — Model Maps as cards (pattern → target) and Route Rules list, with create modals
- **Request Logs** — filterable list (status / free text), 56 px row virtualization, expand-for-error rows, auto-refresh every 10 s
- **Settings** — tabbed layout (General · Routing · Upstream · Retention · Danger), language picker, theme picker, rotating local token, danger zone for log reset

The sidebar is 256 px (collapsible to 72 px, persisted in `localStorage`). A sticky 56 px top bar holds breadcrumbs, language pill (EN / 中) and theme toggle. Dashboard data is read only from the local SQLite `request_logs` and `channels` tables. The API endpoint is `GET /api/dashboard?time_range=1h|24h|7d`.

---

## Settings reference

| Field | Default | Notes |
| --- | --- | --- |
| Listen Host | `127.0.0.1` | Bind address. Use `0.0.0.0` to expose on LAN. |
| Listen Port | `3001` | HTTP port for WebUI and proxy. |
| Open Browser on Start | `true` | Opens the WebUI when the binary launches. |
| Local API Token | random UUID | Bearer token clients send to `localhost`. Persisted. |
| Max Retries | `3` | Channel switch budget per request. |
| Initial Retry Delay (ms) | `500` | Base delay; combined with backoff strategy. |
| Retry Backoff | `exponential` | `fixed`, `linear`, or `exponential`. |
| Upstream Timeout (ms) | `120000` | Per-upstream request timeout. |
| User-Agent | `OxygenRouter/0.1.0` | Sent on every upstream request. |
| Max Concurrent Requests | `64` | In-flight upstream request ceiling. |
| Request Log Retention (days) | `30` | `0` keeps logs forever. |
| Log Level | `info` | `trace` / `debug` / `info` / `warn` / `error`. |

Settings are persisted to:

```text
<exe directory>/config.json
<exe directory>/oxygenrouter.db
```

---

## Development

See [AGENTS.md](./AGENTS.md) for contribution guidelines and [CHANGELOG.md](./CHANGELOG.md) for release history.

### Build

```bash
cargo build              # debug
cargo build --release    # optimized
cargo test               # tests (if any)
```

### Frontend dev

```bash
cd crates/oxygenrouter-webui/web
npm install
npm run dev    # Vite dev server on :5173 with proxy to :3001
```

### End-to-end verification

```bash
python .devlogs/pw_e2e.py
```

This script drives the live `localhost:3001` endpoints and the Vite dev server with Playwright, validating routing, failover, language switching, mobile layout, and the dashboard render.

---

## License

MIT

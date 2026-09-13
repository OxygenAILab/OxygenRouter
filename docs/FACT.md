# FACT.md — OxygenRouter project facts

> **GitHub@OxygenAILab | OxygenAILab@StarsailsClover**

This document records verified project facts. Update it whenever a fact changes.

## Verified facts (as of 2026-08-28)

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

### Implemented proxy endpoints
- `POST /v1/chat/completions` — streaming (SSE) + non-streaming
- `POST /v1/completions`
- `POST /v1/embeddings`
- `POST /v1/images/generations`
- `GET  /v1/models`

### Implemented WebUI pages
- Channels (`/ui/channels`)
- API Keys (`/ui/keys`)
- Routes (`/ui/routes`)
- Logs (`/ui/logs`)
- Settings (`/ui/settings`)

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
- Rust files: ~15
- TypeScript/TSX files: ~10
- CSS files: 1
- Total: ~27 source files

## Open questions
- _None recorded_

## Pending decisions
- _None recorded_

## References
- Inspired by: [NewAPI / one-api](https://github.com/songquanpeng/one-api)
- WebUI style: [OxygenClaw](https://github.com/OxygenAILab/oxygen-claw) (`packages/webui/src/styles/global.css`)
- BC dev convention: `bc-developmentndebugging` skill (v26.0-alpha.5)

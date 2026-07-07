# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Build & Development Commands

```bash
npm run dev              # Vite dev server (frontend only, no Tauri)
npm run tauri dev        # Full Tauri desktop app in dev mode
npm run tauri build      # Production desktop build
npm run build            # Frontend typecheck + vite build
npm run typecheck        # TypeScript type checking only
npm run mcp:build        # Build the MCP server (required for MCP integration in Settings)
npm run mcp:test         # Run MCP server tests
```

## Testing

```bash
npm test                 # All tests: mock tests + real-LLM tests
npm run test:mocks       # vitest run, excludes real-LLM and mcp-server tests
npm run test:llm         # Real LLM integration tests (requires .env.test.local with API keys)
```

Mock tests use `vitest` with `environment: "node"`. Real-LLM tests (`*.real-llm.test.ts`) require actual LLM API credentials in `.env.test.local` (see `src/test-helpers/load-test-env.ts`). There are also `.property.test.ts`, `.scenarios.test.ts`, and `.integration.test.ts` patterns.

## Project Architecture

**LLM Wiki** is a Tauri v2 desktop app that turns documents into an auto-maintained personal knowledge base. It is based on Andrej Karpathy's LLM Wiki pattern.

### Technology Layers

| Layer | Stack |
|-------|-------|
| Desktop shell | Tauri v2 (Rust), `src-tauri/src/lib.rs` — registers commands, plugins, tray, clip server, API server |
| Frontend | React 19 + TypeScript + Vite, `src/main.tsx` → `src/App.tsx` |
| UI | shadcn/ui + Tailwind CSS v4, `components.json` |
| State | Zustand stores in `src/stores/` |
| Editor | Milkdown (ProseMirror WYSIWYG) |
| Graph | sigma.js + graphology + ForceAtlas2 layout |
| Vector DB | LanceDB (Rust, embedded, optional) |
| LLM streaming | Custom fetch-based streaming for OpenAI/Anthropic/Google/Ollama/Custom, plus subprocess-based Claude CLI and Codex CLI providers. All HTTP goes through Tauri's Rust HTTP plugin (not browser fetch) to bypass CORS. |
| i18n | react-i18next, `src/i18n/en.json` + `zh.json` |

### Rust Backend (`src-tauri/src/`)

- `lib.rs` — Tauri app builder: registers all commands, starts clip server (port 19827) and API server (port 19828), tray setup, proxy config, close behavior
- `commands/fs.rs` — File I/O, directory listing, document preprocessing (PDF via pdfium, DOCX/PPTX/XLSX via office_oxide/calamine), cascade delete
- `commands/search.rs` — Tokenized search with CJK bigram support, merges with vector search results
- `commands/vectorstore.rs` — LanceDB vector store operations
- `commands/claude_cli.rs` / `codex_cli.rs` — Subprocess-based LLM transport: spawns CLI, streams stdout back to frontend
- `commands/file_sync.rs` — File watcher for auto-detecting external source file changes
- `commands/extract_images.rs` — PDF/Office image extraction using pdfium
- `api_server.rs` — Local HTTP API at `127.0.0.1:19828` with token auth, project CRUD, search, file read, graph data, source rescan
- `clip_server.rs` — Chrome extension communication server at `127.0.0.1:19827`

### Frontend Architecture (`src/`)

- `App.tsx` — Root: project open/create, state hydration from persisted storage, auto-save, clip watcher, update check, scheduled import setup
- `src/stores/wiki-store.ts` — Central Zustand store: project, LLM config, file tree, search config, embedding config, multimodal config, API config, etc.
- `src/stores/chat-store.ts` — Multi-conversation chat with persistence
- `src/stores/activity-store.ts` — Ingest queue progress visualization
- `src/stores/review-store.ts` — Async human-in-the-loop review items
- `src/stores/lint-store.ts` — Lint check results
- `src/stores/research-store.ts` — Deep research task queue
- `src/lib/` — Core logic (not components). Key modules:
  - `ingest.ts` — Two-step chain-of-thought ingest pipeline (analysis → generation)
  - `ingest-queue.ts` — Persistent serial ingest queue with crash recovery
  - `llm-client.ts` — Unified streaming LLM client with provider dispatch
  - `llm-providers.ts` — Provider-specific API configs and streaming parsers
  - `search.ts` — Multi-phase retrieval: tokenized search → vector search → graph expansion → budget-controlled context assembly
  - `wiki-graph.ts` — Build graph from wikilinks, compute relevance scores (4-signal model)
  - `graph-insights.ts` — Surprising connections, knowledge gaps, bridge node detection
  - `graph-relevance.ts` — 4-signal relevance: direct links, source overlap, Adamic-Adar, type affinity
  - `embedding.ts` — Vector embedding client (OpenAI-compatible endpoint), calls Rust LanceDB commands
  - `deep-research.ts` — Web search (Tavily/SerpApi/SearXNG/Brave/Firecrawl) → LLM synthesis → auto-ingest
  - `lint.ts` — Wiki health checks: dead links, missing frontmatter, orphan pages
  - `web-search.ts` — Multi-provider web search abstraction
  - `scheduled-import.ts` — Periodic folder scanning and auto-ingest
  - `source-lifecycle.ts` — Full lifecycle: delete source → cascade cleanup wiki pages → update index/graph
  - `wiki-cleanup.ts` — Wikilink cleanup after page deletion
  - `frontmatter.ts` — YAML frontmatter parse/write
  - `path-utils.ts` — Cross-platform path normalization
  - `context-budget.ts` — Token budget allocation for LLM context window
  - `project-file-sync.ts` — Source folder auto-watch integration
- `src/components/` — React components:
  - `layout/` — App shell: three-column layout, icon sidebar, file tree, knowledge tree, preview panel, activity panel, research panel
  - `chat/` — Chat input, messages, conversation management
  - `editor/` — Milkdown wiki editor, file preview, frontmatter panel
  - `graph/` — Sigma.js graph visualization with ForceAtlas2 layout (Web Worker)
  - `search/` — Search view with results display
  - `sources/` — Source file browser with progressive loading
  - `review/` — Review queue UI
  - `lint/` — Lint results and fix actions
  - `settings/` — Settings view with sections for LLM, embedding, web search, MinerU, multimodal, network (proxy), API server, source watch, scheduled import, interface, maintenance, changelog
  - `project/` — Welcome screen, create project dialog, template picker

### Ingest Pipeline (Core Feature)

The two-step chain-of-thought ingest is the heart of the app:

1. **Analysis step**: LLM reads source → structured analysis (entities, concepts, connections, contradictions)
2. **Generation step**: LLM takes analysis → generates wiki pages with frontmatter, cross-references, index/log/overview updates

Supports: SHA256 caching, persistent queue with retry, folder import, long-source chunking, image extraction + vision captioning, MinerU cloud parsing (optional), source traceability in frontmatter `sources[]`.

### Wiki Project Structure (created per project)

```
my-wiki/
├── purpose.md          # Goals, key questions, research scope
├── schema.md           # Wiki structure rules, page types
├── raw/sources/        # Uploaded documents (immutable)
├── raw/assets/         # Local images
├── wiki/index.md       # Content catalog
├── wiki/log.md         # Operation history
├── wiki/overview.md    # Global summary (auto-updated)
├── wiki/entities/      # People, organizations, products
├── wiki/concepts/      # Theories, methods, techniques
├── wiki/sources/       # Source summaries
├── wiki/queries/       # Saved chat answers + research
├── wiki/synthesis/     # Cross-source analysis
├── wiki/comparisons/   # Side-by-side comparisons
├── .obsidian/          # Auto-generated Obsidian vault config
└── .llm-wiki/          # App config, chat history, review items, ingest queue
```

### Multi-Provider LLM Support

Providers: `openai`, `anthropic`, `google`, `azure`, `ollama`, `custom`, `minimax`, `claude-code` (subprocess), `codex-cli` (subprocess). Each provider has specific streaming header/body format handling in `src/lib/llm-providers.ts`. The frontend dispatches to the correct streaming path in `src/lib/llm-client.ts`.

### Vector Search (Optional)

Enabled in Settings. Embeddings via any OpenAI-compatible `/v1/embeddings` endpoint, stored in LanceDB (Rust). Chunk-based: text split by `src/lib/text-chunker.ts`, embedded and stored as chunks, searched with RRF fusion. Disabled by default, falls back to tokenized search + graph expansion.

### MCP Server (`mcp-server/`)

Bundled MCP server using `@modelcontextprotocol/sdk`. Calls the local HTTP API at `127.0.0.1:19828`. Tools: list projects, read files, search, graph traversal, source rescan, review export. Built with `npm run mcp:build`, then configurable via Settings → API + MCP.

### Chrome Extension (`extension/`)

Manifest V3 extension using Readability.js + Turndown.js for web clipping. Communicates with the app via `127.0.0.1:19827`. Auto-ingests clipped content.

### CI/CD (`.github/workflows/`)

- `ci.yml` — Cross-platform build check (macOS, Ubuntu, Windows): frontend vite build + Rust cargo build + MCP server build
- `build.yml` — Release builds for macOS (ARM + Intel), Windows (.msi), Linux (.deb / .AppImage)

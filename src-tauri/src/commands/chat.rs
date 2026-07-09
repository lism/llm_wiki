//! Chat pipeline for the local HTTP API.
//!
//! Provides `POST /api/v1/projects/{id}/chat` — a self-contained RAG
//! chat endpoint that reuses the existing `search_project_inner` for
//! retrieval then streams an LLM response via reqwest.  Does NOT depend
//! on the WebView or any frontend store; all config is read from
//! `app-state.json` via the shared `load_app_state` path.
//!
//! Supports the same LLM providers as the desktop chat: OpenAI,
//! Anthropic, Google, Ollama, Custom (OpenAI-compatible), Azure, and
//! MiniMax (Anthropic-wire).  Local subprocess providers (Claude Code
//! CLI, Codex CLI) are *not* supported — the API thread is synchronous
//! and cannot manage long-lived child processes.

use std::collections::BTreeMap;
use std::fs;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::commands::search::{self, ProjectSearchResult, SearchEmbeddingConfig};

// ── request / response types ──────────────────────────────────────

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatRequest {
    pub query: String,
    #[serde(default = "default_true")]
    pub stream: bool,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Reference {
    pub title: String,
    pub path: String,
    pub snippet: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatResponse {
    pub answer: String,
    pub references: Vec<Reference>,
}

// ── LLM config (read from app-state.json) ─────────────────────────

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatLlmConfig {
    pub provider: String,
    #[serde(default)]
    pub api_key: String,
    #[serde(default)]
    pub model: String,
    #[serde(default)]
    pub ollama_url: String,
    #[serde(default)]
    pub custom_endpoint: String,
    #[serde(default)]
    pub azure_api_version: Option<String>,
    #[serde(default)]
    pub api_mode: Option<String>,
    #[serde(default)]
    pub max_context_size: Option<u64>,
}

#[derive(Debug, Clone, Deserialize)]
struct ProviderOverride {
    #[serde(default)]
    api_key: Option<String>,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    ollama_url: Option<String>,
    #[serde(default)]
    custom_endpoint: Option<String>,
    #[serde(default)]
    azure_api_version: Option<String>,
    #[serde(default)]
    api_mode: Option<String>,
    #[serde(default)]
    max_context_size: Option<u64>,
}

/// Resolve the effective LLM config by merging the base `llmConfig`
/// with any per-preset provider override.
pub fn resolve_llm_config(app_state: &Value) -> Option<ChatLlmConfig> {
    let base = app_state.get("llmConfig").cloned().unwrap_or_default();
    if base.as_object().map(|o| o.is_empty()).unwrap_or(true) {
        return None;
    }

    let active_preset_id = app_state
        .get("activePresetId")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();

    let mut config = serde_json::from_value::<ChatLlmConfig>(base.clone()).ok()?;

    // If a preset is active, merge its provider-specific override on
    // top of the base config.  User-set fields in the provider overrides
    // store win over the base preset defaults.
    if !active_preset_id.is_empty() {
        if let Some(overrides) = app_state
            .get("providerConfigs")
            .and_then(|v| v.get(&active_preset_id))
        {
            if let Ok(ov) = serde_json::from_value::<ProviderOverride>(overrides.clone()) {
                if let Some(key) = ov.api_key.filter(|s| !s.is_empty()) {
                    config.api_key = key;
                }
                if let Some(model) = ov.model.filter(|s| !s.is_empty()) {
                    config.model = model;
                }
                if let Some(url) = ov.ollama_url.filter(|s| !s.is_empty()) {
                    config.ollama_url = url;
                }
                if let Some(ep) = ov.custom_endpoint.filter(|s| !s.is_empty()) {
                    config.custom_endpoint = ep;
                }
                if let Some(ver) = ov.azure_api_version {
                    config.azure_api_version = Some(ver);
                }
                if let Some(mode) = ov.api_mode {
                    config.api_mode = Some(mode);
                }
                if let Some(ctx) = ov.max_context_size {
                    config.max_context_size = Some(ctx);
                }
            }
        }
    }

    let key_empty = config.api_key.trim().is_empty();
    let needs_key = !matches!(
        config.provider.as_str(),
        "ollama" | "claude-code" | "codex-cli" | ""
    );
    if needs_key && key_empty {
        return None;
    }

    Some(config)
}

// ── system context loading ────────────────────────────────────────

struct SystemContext {
    purpose: String,
    overview: String,
    index: String,
}

fn load_system_context(project_path: &str) -> SystemContext {
    let read = |rel: &str| -> String {
        let path = std::path::Path::new(project_path).join(rel);
        fs::read_to_string(&path).unwrap_or_default()
    };

    SystemContext {
        purpose: read("purpose.md"),
        overview: read("wiki/overview.md"),
        index: read("wiki/index.md"),
    }
}

// ── context budget (mirrors frontend computeContextBudget) ──────────

const DEFAULT_MAX_CTX: u64 = 204_800;
const RESPONSE_RESERVE_FRAC: f64 = 0.15;
const INDEX_BUDGET_FRAC: f64 = 0.05;
const PAGE_BUDGET_FRAC: f64 = 0.5;
const PER_PAGE_FRAC: f64 = 0.3;
const PER_PAGE_FLOOR: usize = 5_000;

struct ContextBudget {
    index_budget: usize,
    page_budget: usize,
    max_page_size: usize,
}

fn compute_context_budget(max_context_size: Option<u64>) -> ContextBudget {
    let max_ctx = max_context_size
        .filter(|&n| n > 0)
        .unwrap_or(DEFAULT_MAX_CTX) as usize;

    let index_budget = (max_ctx as f64 * INDEX_BUDGET_FRAC) as usize;
    let page_budget = (max_ctx as f64 * PAGE_BUDGET_FRAC) as usize;
    let max_page_size = page_budget
        .min(((page_budget as f64 * PER_PAGE_FRAC) as usize).max(PER_PAGE_FLOOR));

    ContextBudget {
        index_budget,
        page_budget,
        max_page_size,
    }
}

// ── language directive ─────────────────────────────────────────────

/// Build a language directive matching the frontend's
/// `buildLanguageDirective`.  Detects query language via simple CJK
/// heuristics and emits a mandatory-output-language instruction.
fn build_language_directive(query: &str) -> String {
    let has_cjk = query
        .chars()
        .any(|c| ('\u{3400}'..='\u{9fff}').contains(&c));
    let lang = if has_cjk { "Chinese" } else { "English" };

    format!(
        "## ⚠️ MANDATORY OUTPUT LANGUAGE: {lang}\n\n\
         Write surrounding natural-language prose in **{lang}**.\n\
         All generated prose, including prose titles and section headings, must be in {lang}.\n\
         Do not translate, transliterate, or describe proper nouns and technical identifiers \
         unless the source already uses a well-established localized form.\n\
         Preserve organization names, product names, model names, dataset names, \
         tool/library names, acronyms, code identifiers, file names, URLs, paper titles, \
         citation strings, and technical terms that have no widely-used localized equivalent \
         in their standard original form.\n\
         The source material or wiki content may be in a different language; use it as \
         evidence, but keep generated prose in {lang}.\n\
         This language rule overrides weaker style instructions, but it does not override \
         the proper-noun and technical-identifier preservation rule above.",
    )
}

// ── prompt assembly ───────────────────────────────────────────────

/// Escape special XML/HTML characters so file content embedded inside
/// `<context>` blocks does not break the XML structure.
fn escape_xml(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// Trim the wiki index to entries most relevant to the query.  Keeps
/// heading lines (##) and lines whose lowercase text contains at least
/// one query token, up to `max_chars`.
fn trim_relevant_index(raw: &str, query: &str, max_chars: usize) -> String {
    if raw.is_empty() || raw.len() <= max_chars {
        return raw.to_string();
    }
    let tokens: Vec<String> = query
        .to_lowercase()
        .split(|c: char| c.is_whitespace() || c.is_ascii_punctuation())
        .map(|t| t.trim().to_string())
        .filter(|t| t.len() > 1)
        .take(20)
        .collect();

    let mut kept = Vec::new();
    let mut size = 0usize;
    for line in raw.lines() {
        let is_header = line.starts_with("##");
        let lower = line.to_lowercase();
        let is_relevant = tokens.is_empty()
            || tokens.iter().any(|t| lower.contains(t.as_str()));
        if !is_header && !is_relevant {
            continue;
        }
        if size + line.len() + 1 > max_chars {
            continue;
        }
        size += line.len() + 1;
        kept.push(line);
    }
    if kept.is_empty() {
        let end = raw
            .char_indices()
            .take(max_chars.saturating_sub(40))
            .last()
            .map(|(i, _)| i)
            .unwrap_or(0);
        format!("{}…\n[...index truncated...]", &raw[..end])
    } else {
        format!(
            "{}\n\n[...index trimmed to relevant entries...]",
            kept.join("\n")
        )
    }
}

fn assemble_system_prompt(
    ctx: &SystemContext,
    search_results: &[ProjectSearchResult],
    query: &str,
    budget: &ContextBudget,
) -> String {
    let mut parts: Vec<String> = Vec::new();

    // Core identity
    parts.push(
        "You are a knowledgeable wiki assistant. Answer using the \
         retrieved context below.\n\
         If the retrieved context is insufficient, say what is missing \
         instead of inventing facts.\n\
         Keep subject boundaries strict: do not apply a claim, limitation, \
         evaluation, benchmark result, or recommendation about one entity \
         to another subject just because they share keywords.\n\
         If retrieved context discusses multiple subjects, attribute each \
         claim to the exact subject named in that context block; when \
         uncertain, state the uncertainty instead of generalizing."
            .to_string(),
    );

    // Language directive
    parts.push(build_language_directive(query));

    // Wiki Purpose
    if !ctx.purpose.trim().is_empty() {
        parts.push(format!(
            "## Wiki Purpose\n\n{}\n",
            ctx.purpose.trim()
        ));
    }

    // Wiki Overview
    let overview_trimmed = trim_for_budget(
        ctx.overview.trim(),
        ((budget.page_budget as f64) * 0.25) as usize,
    );
    if !overview_trimmed.is_empty() {
        parts.push(format!(
            "## Wiki Overview\n\n{}\n",
            overview_trimmed
        ));
    }

    // Wiki Index (relevance-trimmed)
    let index_trimmed = trim_relevant_index(
        ctx.index.trim(),
        query,
        ((budget.index_budget as f64) * 0.7) as usize,
    );
    if !index_trimmed.is_empty() {
        parts.push(format!(
            "## Wiki Index\n\n{}\n",
            index_trimmed
        ));
    }

    // Retrieved pages in XML context blocks, 1-indexed
    if !search_results.is_empty() {
        parts.push("## Retrieved Context\n".to_string());
        let mut used_chars = 0usize;
        let mut local_index: usize = 0;

        for result in search_results {
            if used_chars >= budget.page_budget {
                break;
            }
            local_index += 1;
            let content = result.content.as_deref().unwrap_or("");
            let truncated = trim_for_budget(content, budget.max_page_size);
            if truncated.is_empty() {
                continue;
            }
            if used_chars + truncated.len() > budget.page_budget {
                continue;
            }
            used_chars += truncated.len();

            let title_escaped = escape_xml(&result.title);
            let path_escaped = escape_xml(&result.path);
            let content_escaped = escape_xml(&truncated);
            let block = format!(
                "<context id=\"{local_index}\" source=\"wiki\" kind=\"wiki\" \
                 title=\"{title_escaped}\" path=\"{path_escaped}\">\n\
                 {content_escaped}\n\
                 </context>"
            );
            parts.push(block);
        }
    }

    // Citation instructions (matching in-app format)
    parts.push(
        "\n\
         Use [[wikilink]] syntax for local wiki pages when relevant.\n\
         When a sentence or bullet uses retrieved context, include an inline \
         citation immediately after that claim.\n\
         Cite local context blocks with [1], [2].\n\
         Do not rely on a separate References panel as a substitute for \
         inline citations in the answer body.\n\
         At the VERY END of your response, add a hidden comment listing \
         which local page numbers you used:\n\
           <!-- cited: 1, 3 -->"
            .to_string(),
    );

    parts.join("\n\n")
}

fn trim_for_budget(content: &str, max_chars: usize) -> String {
    if content.len() <= max_chars {
        return content.to_string();
    }
    let end = content
        .char_indices()
        .take(max_chars.saturating_sub(20))
        .last()
        .map(|(i, _)| i)
        .unwrap_or(0);
    format!("{}…\n[...truncated...]", &content[..end])
}

// ── citation parsing ──────────────────────────────────────────────

/// Extract citations like `[1]`, `[2]` from the LLM answer.
/// 1-indexed to match the in-app chat numbering.
fn parse_citations(answer: &str, search_results: &[ProjectSearchResult]) -> Vec<Reference> {
    let mut seen = BTreeMap::new();
    let bytes = answer.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'[' {
            let start = i;
            i += 1;
            let mut num = 0usize;
            let mut digits = 0;
            while i < bytes.len() && bytes[i].is_ascii_digit() {
                num = num.saturating_mul(10).saturating_add((bytes[i] - b'0') as usize);
                digits += 1;
                i += 1;
            }
            // 1-indexed: valid range is 1..=search_results.len()
            if digits > 0
                && i < bytes.len()
                && bytes[i] == b']'
                && num >= 1
                && num <= search_results.len()
            {
                let result_idx = num - 1; // convert to 0-indexed for array access
                seen.entry(num).or_insert_with(|| {
                    let r = &search_results[result_idx];
                    Reference {
                        title: r.title.clone(),
                        path: r.path.clone(),
                        snippet: r.snippet.clone(),
                    }
                });
            }
            i = start + 1; // continue scanning after the opening bracket
        } else {
            i += 1;
        }
    }

    // Return references in citation order
    let mut ordered: Vec<(usize, Reference)> = seen.into_iter().collect();
    ordered.sort_by_key(|(k, _)| *k);
    ordered.into_iter().map(|(_, r)| r).collect()
}

// ── LLM streaming ─────────────────────────────────────────────────

/// Token emitted by the streaming parser.
#[derive(Debug, Clone)]
pub enum StreamEvent {
    Token(String),
    Done,
    Error(String),
}

/// Provider-specific streaming config.
pub struct ProviderStreamConfig {
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body_json: Value,
    /// Parse one SSE / JSON-lines chunk, returning tokens.
    pub parse_line: fn(&str) -> Option<String>,
}

fn build_provider_config(
    cfg: &ChatLlmConfig,
    system_prompt: &str,
    user_query: &str,
) -> Result<ProviderStreamConfig, String> {
    let provider = cfg.provider.as_str();

    match provider {
        "openai" => {
            let url = "https://api.openai.com/v1/chat/completions".to_string();
            let body = json!({
                "model": cfg.model,
                "messages": [
                    {"role": "system", "content": system_prompt},
                    {"role": "user",   "content": user_query}
                ],
                "stream": true
            });
            Ok(ProviderStreamConfig {
                url,
                headers: vec![
                    ("Content-Type".to_string(), "application/json".to_string()),
                    ("Authorization".to_string(), format!("Bearer {}", cfg.api_key.trim())),
                ],
                body_json: body,
                parse_line: parse_openai_line,
            })
        }

        "anthropic" => {
            let url = build_anthropic_url("https://api.anthropic.com");
            let body = json!({
                "model": cfg.model,
                "system": system_prompt,
                "messages": [
                    {"role": "user", "content": user_query}
                ],
                "max_tokens": 4096,
                "stream": true
            });
            // Anthropic requires max_tokens field — already set above
            let headers = build_anthropic_headers(cfg.api_key.trim(), &url);
            Ok(ProviderStreamConfig {
                url,
                headers,
                body_json: body,
                parse_line: parse_anthropic_line,
            })
        }

        "google" => {
            let model = cfg.model.trim();
            let model_enc = urlencoding(model);
            let url = format!(
                "https://generativelanguage.googleapis.com/v1beta/models/{model_enc}:streamGenerateContent?alt=sse"
            );
            let body = json!({
                "systemInstruction": {
                    "parts": [{"text": system_prompt}]
                },
                "contents": [
                    {"role": "user", "parts": [{"text": user_query}]}
                ]
            });
            Ok(ProviderStreamConfig {
                url,
                headers: vec![
                    ("Content-Type".to_string(), "application/json".to_string()),
                    ("x-goog-api-key".to_string(), cfg.api_key.trim().to_string()),
                ],
                body_json: body,
                parse_line: parse_google_line,
            })
        }

        "azure" => {
            let endpoint = cfg.custom_endpoint.trim().trim_end_matches('/');
            let api_version = cfg
                .azure_api_version
                .as_deref()
                .unwrap_or("2024-10-21");
            // Build Azure URL: {endpoint}/openai/deployments/{model}/chat/completions?api-version=...
            let url = if endpoint.contains("deployments") {
                format!("{endpoint}/chat/completions?api-version={api_version}")
            } else {
                let model = cfg.model.trim();
                format!("{endpoint}/openai/deployments/{model}/chat/completions?api-version={api_version}")
            };
            let body = json!({
                "messages": [
                    {"role": "system", "content": system_prompt},
                    {"role": "user",   "content": user_query}
                ],
                "stream": true
            });
            Ok(ProviderStreamConfig {
                url,
                headers: vec![
                    ("Content-Type".to_string(), "application/json".to_string()),
                    ("api-key".to_string(), cfg.api_key.trim().to_string()),
                ],
                body_json: body,
                parse_line: parse_openai_line,
            })
        }

        "ollama" => {
            let base = cfg.ollama_url.trim().trim_end_matches('/');
            let url = if base.ends_with("/v1/chat/completions") {
                base.to_string()
            } else if base.ends_with("/v1") {
                format!("{base}/chat/completions")
            } else {
                format!("{base}/v1/chat/completions")
            };
            let body = json!({
                "model": cfg.model,
                "messages": [
                    {"role": "system", "content": system_prompt},
                    {"role": "user",   "content": user_query}
                ],
                "stream": true
            });
            Ok(ProviderStreamConfig {
                url,
                headers: vec![
                    ("Content-Type".to_string(), "application/json".to_string()),
                    ("Origin".to_string(), "http://localhost".to_string()),
                ],
                body_json: body,
                parse_line: parse_openai_line,
            })
        }

        "minimax" => {
            let base = cfg.custom_endpoint.trim();
            let endpoint = if base.is_empty() {
                "https://api.minimax.io/anthropic".to_string()
            } else {
                base.to_string()
            };
            let url = build_anthropic_url(&endpoint);
            let headers = build_anthropic_headers(cfg.api_key.trim(), &url);
            let body = json!({
                "model": cfg.model,
                "system": system_prompt,
                "messages": [
                    {"role": "user", "content": user_query}
                ],
                "max_tokens": 4096,
                "stream": true
            });
            Ok(ProviderStreamConfig {
                url,
                headers,
                body_json: body,
                parse_line: parse_anthropic_line,
            })
        }

        "custom" => {
            let base = cfg.custom_endpoint.trim().trim_end_matches('/');
            if base.is_empty() {
                return Err("customEndpoint is required for the 'custom' provider".to_string());
            }
            let mode = cfg.api_mode.as_deref().unwrap_or("chat_completions");
            if mode == "anthropic_messages" {
                let url = build_anthropic_url(base);
                let headers = build_anthropic_headers(cfg.api_key.trim(), &url);
                let body = json!({
                    "model": cfg.model,
                    "system": system_prompt,
                    "messages": [
                        {"role": "user", "content": user_query}
                    ],
                    "max_tokens": 4096,
                    "stream": true
                });
                Ok(ProviderStreamConfig {
                    url,
                    headers,
                    body_json: body,
                    parse_line: parse_anthropic_line,
                })
            } else {
                let url = if base.ends_with("/chat/completions") {
                    base.to_string()
                } else {
                    format!("{base}/chat/completions")
                };
                let api_key = cfg.api_key.trim();
                let mut headers = vec![
                    ("Content-Type".to_string(), "application/json".to_string()),
                ];
                if !api_key.is_empty() {
                    headers.push(("Authorization".to_string(), format!("Bearer {api_key}")));
                }
                let mut body = json!({
                    "messages": [
                        {"role": "system", "content": system_prompt},
                        {"role": "user",   "content": user_query}
                    ],
                    "stream": true
                });
                body["model"] = json!(cfg.model);
                Ok(ProviderStreamConfig {
                    url,
                    headers,
                    body_json: body,
                    parse_line: parse_openai_line,
                })
            }
        }

        other => Err(format!(
            "Provider '{other}' is not supported by the chat API. \
             Supported: openai, anthropic, google, azure, ollama, minimax, custom"
        )),
    }
}

fn build_anthropic_headers(api_key: &str, url: &str) -> Vec<(String, String)> {
    let requires_bearer = is_anthropic_bearer_auth_url(url);
    let mut headers = vec![
        ("Content-Type".to_string(), "application/json".to_string()),
        ("anthropic-version".to_string(), "2023-06-01".to_string()),
    ];
    if requires_bearer {
        headers.push(("Authorization".to_string(), format!("Bearer {api_key}")));
    } else {
        headers.push(("x-api-key".to_string(), api_key.to_string()));
        headers.push((
            "anthropic-dangerous-direct-browser-access".to_string(),
            "true".to_string(),
        ));
    }
    headers
}

fn is_anthropic_bearer_auth_url(url: &str) -> bool {
    let lower = url.to_lowercase();
    lower.starts_with("https://api.minimax.io/anthropic")
        || lower.starts_with("https://api.minimaxi.com/anthropic")
        || lower.starts_with("https://coding.dashscope.aliyuncs.com/apps/anthropic")
        || lower.contains("token-plan-cn.xiaomimimo.com/anthropic")
        || lower.starts_with("https://api.kimi.com/coding")
        || lower.starts_with("https://api.moonshot.ai/anthropic")
        || lower.starts_with("https://api.moonshot.cn/anthropic")
}

fn build_anthropic_url(base: &str) -> String {
    let trimmed = base.trim_end_matches('/');
    if trimmed.ends_with("/v1/messages") || trimmed.ends_with("/v2/messages") {
        return trimmed.to_string();
    }
    if let Some(pos) = trimmed.rfind("/v1") {
        if trimmed[pos..].len() <= 4 {
            return format!("{}/messages", &trimmed[..pos + 3]);
        }
    }
    format!("{trimmed}/v1/messages")
}

fn urlencoding(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => {
                out.push_str(&format!("%{:02X}", b));
            }
        }
    }
    out
}

// ── stream parsers ────────────────────────────────────────────────

fn parse_openai_line(line: &str) -> Option<String> {
    let data = line.strip_prefix("data:")?.trim();
    if data == "[DONE]" {
        return None;
    }
    let parsed: Value = serde_json::from_str(data).ok()?;
    parsed
        .get("choices")?
        .as_array()?
        .first()?
        .get("delta")?
        .get("content")?
        .as_str()
        .map(|s| s.to_string())
}

fn parse_anthropic_line(line: &str) -> Option<String> {
    let data = line.strip_prefix("data:")?.trim();
    if data == "[DONE]" {
        return None;
    }
    let parsed: Value = serde_json::from_str(data).ok()?;
    let typ = parsed.get("type")?.as_str()?;
    if typ == "content_block_delta" {
        let delta = parsed.get("delta")?;
        if delta.get("type")?.as_str()? == "text_delta" {
            return delta.get("text")?.as_str().map(|s| s.to_string());
        }
    }
    // Fallback: some proxies return message-complete events
    if typ == "message" {
        if let Some(content) = parsed.get("content").and_then(|c| c.as_array()) {
            let text: String = content
                .iter()
                .filter_map(|b| b.get("text").and_then(|t| t.as_str()))
                .collect::<Vec<_>>()
                .join("");
            if !text.is_empty() {
                return Some(text);
            }
        }
    }
    None
}

fn parse_google_line(line: &str) -> Option<String> {
    let data = line.strip_prefix("data:")?.trim();
    let parsed: Value = serde_json::from_str(data).ok()?;
    let parts = parsed
        .get("candidates")?
        .as_array()?
        .first()?
        .get("content")?
        .get("parts")?
        .as_array()?;
    let mut out = String::new();
    for part in parts {
        if part.get("thought").and_then(|t| t.as_bool()).unwrap_or(false) {
            continue;
        }
        if let Some(text) = part.get("text").and_then(|t| t.as_str()) {
            out.push_str(text);
        }
    }
    if out.is_empty() {
        None
    } else {
        Some(out)
    }
}

// ── main entry point ──────────────────────────────────────────────

/// Run the full RAG chat pipeline: search → assemble prompt → stream LLM.
///
/// Returns `(ChatResponse, Vec<StreamEvent>)` where `StreamEvent::Token`
/// events are emitted as they arrive.  Callers can collect all tokens
/// for a non-streaming response, or forward them to an SSE client.
pub async fn run_chat_query(
    project_path: &str,
    query: &str,
    config: &ChatLlmConfig,
    embedding_config: Option<SearchEmbeddingConfig>,
) -> Result<(ChatResponse, Vec<StreamEvent>), String> {
    // 1. Compute context budget from the model's max context window
    let budget = compute_context_budget(config.max_context_size);

    // 2. Resolve query embedding (for optional vector search)
    let query_embedding = search::resolve_query_embedding(query, None, embedding_config.clone())
        .await
        .unwrap_or_else(|e| {
            eprintln!("[Chat] embedding unavailable, using keyword-only search: {e}");
            None
        });

    // 3. Search — include_content so we can trim to budget in assembly
    let search = search::search_project_inner(
        project_path.to_string(),
        query.to_string(),
        10,
        true, // include_content
        query_embedding,
    )
    .await?;

    // 4. Load system context (purpose, overview, index)
    let ctx = load_system_context(project_path);

    // 5. Assemble system prompt with dynamic budget, language directive,
    //    XML context blocks, and 1-indexed citations.
    let system_prompt = assemble_system_prompt(&ctx, &search.results, query, &budget);

    // 6. Build provider config
    let provider = build_provider_config(config, &system_prompt, query)?;

    // 7. Stream LLM
    let (answer, events) = stream_llm(&provider).await?;

    // 8. Parse citations (1-indexed: [1], [2], …)
    let references = parse_citations(&answer, &search.results);

    Ok((
        ChatResponse { answer, references },
        events,
    ))
}

/// Stream an LLM response via reqwest, returning the full concatenated
/// answer text plus all individual stream events.
async fn stream_llm(
    cfg: &ProviderStreamConfig,
) -> Result<(String, Vec<StreamEvent>), String> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(120))
        .build()
        .map_err(|e| format!("HTTP client error: {e}"))?;

    let mut req = client.post(&cfg.url);
    for (name, value) in &cfg.headers {
        req = req.header(name.as_str(), value.as_str());
    }

    let resp = req
        .json(&cfg.body_json)
        .send()
        .await
        .map_err(|e| format!("LLM request failed: {e}"))?;

    let status = resp.status();
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        let snippet: String = body.chars().take(300).collect();
        return Err(format!("LLM API HTTP {status}: {snippet}"));
    }

    let mut answer = String::new();
    let mut events: Vec<StreamEvent> = Vec::new();
    let mut stream = resp.bytes_stream();

    use futures::StreamExt;
    let mut buffer = String::new();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| format!("Stream read error: {e}"))?;
        let text = String::from_utf8_lossy(&chunk);
        buffer.push_str(&text);

        // Process complete lines from the buffer
        while let Some(newline_pos) = buffer.find('\n') {
            let line = buffer[..newline_pos].trim().to_string();
            buffer = buffer[newline_pos + 1..].to_string();

            if line.is_empty() {
                continue;
            }

            if let Some(token) = (cfg.parse_line)(&line) {
                answer.push_str(&token);
                events.push(StreamEvent::Token(token));
            }
        }
    }

    // Flush any remaining buffer content
    let remaining = buffer.trim().to_string();
    if !remaining.is_empty() {
        if let Some(token) = (cfg.parse_line)(&remaining) {
            answer.push_str(&token);
            events.push(StreamEvent::Token(token));
        }
    }

    events.push(StreamEvent::Done);
    Ok((answer, events))
}

// ── public helpers for ingest ─────────────────────────────────────

/// Build a `ProviderStreamConfig` for a raw system + user prompt pair.
/// Used by the ingest pipeline (and any other callers that just want
/// to call the LLM without the chat-agent search/context assembly).
pub fn build_provider_config_for_ingest(
    config: &ChatLlmConfig,
    system_prompt: &str,
    user_prompt: &str,
) -> Result<ProviderStreamConfig, String> {
    build_provider_config(config, system_prompt, user_prompt)
}

/// Stream an LLM call and return the complete collected response text
/// plus all stream events.  Used by non-chat callers (ingest, etc.).
pub async fn stream_llm_raw(
    cfg: &ProviderStreamConfig,
) -> Result<(String, Vec<StreamEvent>), String> {
    stream_llm(cfg).await
}

// ── tests ─────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_openai_line_extracts_content() {
        let line = r#"data: {"choices":[{"delta":{"content":"hello"}}]}"#;
        assert_eq!(parse_openai_line(line), Some("hello".to_string()));
    }

    #[test]
    fn parse_openai_line_ignores_done() {
        assert_eq!(parse_openai_line("data: [DONE]"), None);
    }

    #[test]
    fn parse_openai_line_ignores_non_data_lines() {
        assert_eq!(parse_openai_line(": heartbeat"), None);
    }

    #[test]
    fn parse_anthropic_line_extracts_text_delta() {
        let line = r#"data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"world"}}"#;
        assert_eq!(parse_anthropic_line(line), Some("world".to_string()));
    }

    #[test]
    fn parse_anthropic_line_ignores_done() {
        assert_eq!(parse_anthropic_line("data: [DONE]"), None);
    }

    #[test]
    fn parse_google_line_extracts_parts() {
        let line = r#"data: {"candidates":[{"content":{"parts":[{"text":"bonjour"}]}}]}"#;
        assert_eq!(parse_google_line(line), Some("bonjour".to_string()));
    }

    #[test]
    fn parse_google_line_skips_thought_parts() {
        let line = r#"data: {"candidates":[{"content":{"parts":[{"text":"think","thought":true},{"text":"answer"}]}}]}"#;
        assert_eq!(parse_google_line(line), Some("answer".to_string()));
    }

    #[test]
    fn parse_citations_extracts_numbered_refs() {
        let results = vec![
            ProjectSearchResult {
                path: "wiki/concepts/attention.md".to_string(),
                title: "Attention".to_string(),
                snippet: "A mechanism...".to_string(),
                title_match: true,
                score: 10.0,
                vector_score: None,
                images: vec![],
                content: None,
            },
            ProjectSearchResult {
                path: "wiki/concepts/transformer.md".to_string(),
                title: "Transformer".to_string(),
                snippet: "A model architecture...".to_string(),
                title_match: false,
                score: 5.0,
                vector_score: None,
                images: vec![],
                content: None,
            },
        ];

        // 1-indexed: [1] = first result, [2] = second
        let refs = parse_citations(
            "Attention [1] is key. The Transformer [2] uses it. See also [1].",
            &results,
        );
        assert_eq!(refs.len(), 2);
        assert_eq!(refs[0].title, "Attention");
        assert_eq!(refs[1].title, "Transformer");
    }

    #[test]
    fn parse_citations_ignores_out_of_range() {
        let results = vec![ProjectSearchResult {
            path: "a.md".to_string(),
            title: "A".to_string(),
            snippet: "...".to_string(),
            title_match: false,
            score: 0.0,
            vector_score: None,
            images: vec![],
            content: None,
        }];
        // [0] is below 1-indexed range, [5] is beyond 1 result
        let refs = parse_citations("See [0] and [5].", &results);
        assert_eq!(refs.len(), 0);
        // [1] is valid (1-indexed, only result)
        let refs2 = parse_citations("See [1].", &results);
        assert_eq!(refs2.len(), 1);
    }

    #[test]
    fn urlencoding_handles_slashes() {
        let encoded = urlencoding("models/gemini-2.0-flash");
        assert!(!encoded.contains('/'));
    }

    #[test]
    fn build_anthropic_url_preserves_full_path() {
        let url = build_anthropic_url("https://api.anthropic.com/v1/messages");
        assert_eq!(url, "https://api.anthropic.com/v1/messages");
    }

    #[test]
    fn build_anthropic_url_appends_path() {
        let url = build_anthropic_url("https://api.minimax.io/anthropic");
        assert_eq!(url, "https://api.minimax.io/anthropic/v1/messages");
    }

    #[test]
    fn compute_context_budget_defaults_and_scales() {
        // Default (no explicit config) → 200K chars
        let b = compute_context_budget(None);
        let default_ctx = DEFAULT_MAX_CTX as usize;
        assert_eq!(b.page_budget, (default_ctx as f64 * PAGE_BUDGET_FRAC) as usize);
        assert!(b.max_page_size >= PER_PAGE_FLOOR);
        assert!(b.max_page_size <= b.page_budget);

        // Tiny config: pageBudget < PER_PAGE_FLOOR → per-page cap clamped
        let tiny = compute_context_budget(Some(10_000));
        assert_eq!(tiny.max_page_size, tiny.page_budget);
    }

    #[test]
    fn language_directive_detects_cjk() {
        let en = build_language_directive("What is attention?");
        assert!(en.contains("English"));
        assert!(!en.contains("Chinese"));

        let zh = build_language_directive("什么是注意力机制？");
        assert!(zh.contains("Chinese"));
    }

    #[test]
    fn escape_xml_handles_special_chars() {
        let input = r#"<tag attr="val">&text"#;
        let escaped = escape_xml(input);
        assert!(escaped.contains("&lt;"));
        assert!(escaped.contains("&gt;"));
        assert!(escaped.contains("&quot;"));
        assert!(escaped.contains("&amp;"));
        assert!(!escaped.contains('<'));
    }

    #[test]
    fn trim_relevant_index_prefers_relevant_lines() {
        let raw = "## Concepts\nattention\n## Entities\nBERT\n## Methods\nrandom";
        let trimmed = trim_relevant_index(raw, "attention mechanism", 200);
        assert!(trimmed.contains("## Concepts"));
        assert!(trimmed.contains("## Methods"));
    }

    #[test]
    fn resolve_llm_config_merges_provider_overrides() {
        let state = json!({
            "llmConfig": {
                "provider": "openai",
                "apiKey": "sk-base",
                "model": "gpt-4o",
                "ollamaUrl": "",
                "customEndpoint": ""
            },
            "activePresetId": "preset1",
            "providerConfigs": {
                "preset1": {
                    "model": "gpt-4o-mini",
                    "apiKey": "sk-override"
                }
            }
        });
        let cfg = resolve_llm_config(&state).unwrap();
        assert_eq!(cfg.provider, "openai");
        assert_eq!(cfg.api_key, "sk-override");
        assert_eq!(cfg.model, "gpt-4o-mini");
    }
}

//! Ingest pipeline — read a source file, call the LLM twice (analysis
//! then generation), and write the resulting wiki pages.
//!
//! This is a partial port of the TypeScript `src/lib/ingest.ts` with
//! the same two-step Chain-of-Thought flow.  Long-source chunking,
//! MinerU cloud parsing, image extraction / captioning, and the
//! review-stage-3 suggestion pass are NOT ported yet.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::commands::chat::{self, ChatLlmConfig, StreamEvent};

// ── source identity ──────────────────────────────────────────────

/// Extract the stable source identity from a raw/sources/… path.
/// Mirror of TypeScript `sourceIdentityForPath`.
pub fn source_identity(source_path: &str) -> String {
    let sp = normalize(source_path);
    if let Some(pos) = sp.to_lowercase().find("/raw/sources/") {
        let start = pos + "/raw/sources/".len();
        return sp[start..].to_string();
    }
    // Fallback: use the file name
    Path::new(&sp)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or(&sp)
        .to_string()
}

/// Slug for the source-summary wiki page.
pub fn source_summary_slug(identity: &str) -> String {
    let without_ext = identity
        .rsplitn(2, '.')
        .last()
        .unwrap_or(identity);
    let slug = without_ext
        .split('/')
        .last()
        .unwrap_or(without_ext);
    // Truncate, lowercase, replace spaces
    let mut out: String = slug
        .chars()
        .take(100)
        .collect::<String>()
        .to_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() || c == '-' { c } else { '-' })
        .collect();
    // Collapse consecutive dashes
    while out.contains("--") {
        out = out.replace("--", "-");
    }
    out.trim_matches('-').to_string()
}

fn normalize(s: &str) -> String {
    s.replace('\\', "/")
}

// ── SHA-256 cache ────────────────────────────────────────────────

#[derive(Debug, Default, Serialize, Deserialize)]
struct CacheData {
    entries: BTreeMap<String, CacheEntry>,
}

#[derive(Debug, Serialize, Deserialize)]
struct CacheEntry {
    hash: String,
    #[serde(rename = "timestamp")]
    _timestamp: u64,
    #[serde(rename = "filesWritten")]
    _files_written: Vec<String>,
}

fn cache_path(project_path: &str) -> PathBuf {
    Path::new(project_path).join(".llm-wiki/ingest-cache.json")
}

fn load_cache(project_path: &str) -> CacheData {
    let path = cache_path(project_path);
    fs::read_to_string(&path)
        .ok()
        .and_then(|raw| serde_json::from_str(&raw).ok())
        .unwrap_or_default()
}

fn save_cache(project_path: &str, cache: &CacheData) {
    let path = cache_path(project_path);
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    if let Ok(json) = serde_json::to_string_pretty(cache) {
        let _ = fs::write(&path, json);
    }
}

fn sha256_hex(content: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(content.as_bytes());
    format!("{:x}", hasher.finalize())
}

// ── language directive ───────────────────────────────────────────

fn language_directive(source_content: &str) -> String {
    let has_cjk = source_content
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
         in their standard original form.",
    )
}

fn wiki_today() -> String {
    let now = chrono::Local::now();
    now.format("%Y-%m-%d").to_string()
}

/// Allowed wiki page types for the generation prompt.
const GENERATION_WIKI_TYPES: &[&str] = &[
    "entity", "concept", "source", "query", "synthesis", "comparison",
    "index", "overview", "log",
];

// ── Step 1: Analysis prompt ─────────────────────────────────────

fn build_analysis_prompt(
    purpose: &str,
    index: &str,
    schema: &str,
    source_content: &str,
    folder_context: Option<&str>,
) -> String {
    let mut parts: Vec<String> = vec![
        "You are an expert research analyst. Read the source document and produce a structured analysis.".into(),
        "Do not output chain-of-thought, hidden reasoning, or a thinking transcript. Reason internally and write only the concise final analysis.".into(),
        String::new(),
        language_directive(source_content),
        String::new(),
        "Your analysis should cover:".into(),
        String::new(),
        "## Key Entities".into(),
        "List people, organizations, products, datasets, tools mentioned. For each:".into(),
        "- Name and type".into(),
        "- Role in the source (central vs. peripheral)".into(),
        "- Whether it likely already exists in the wiki (check the index)".into(),
        String::new(),
        "## Key Concepts".into(),
        "List theories, methods, techniques, phenomena. For each:".into(),
        "- Name and brief definition".into(),
        "- Why it matters in this source".into(),
        "- Whether it likely already exists in the wiki".into(),
        String::new(),
        "## Main Arguments & Findings".into(),
        "- What are the core claims or results?".into(),
        "- What evidence supports them?".into(),
        "- How strong is the evidence?".into(),
        "- Which named subject is each claim about? Do not transfer claims, limits, or evaluations from one entity/model/product/method to another just because they share keywords.".into(),
        String::new(),
        "## Connections to Existing Wiki".into(),
        "- What existing pages does this source relate to?".into(),
        "- Does it strengthen, challenge, or extend existing knowledge?".into(),
        String::new(),
        "## Contradictions & Tensions".into(),
        "- Does anything in this source conflict with existing wiki content?".into(),
        "- Are there internal tensions or caveats?".into(),
        String::new(),
        "## Recommendations".into(),
        "- What wiki pages should be created or updated?".into(),
        "- What should be emphasized vs. de-emphasized?".into(),
        "- Any open questions worth flagging for the user?".into(),
        String::new(),
        "Be thorough but concise. Focus on what's genuinely important.".into(),
    ];

    if let Some(fc) = folder_context {
        if !fc.is_empty() {
            parts.push(format!(
                "If a folder context is provided, use it as a hint for categorization — \
                 the folder structure often reflects the user's organizational intent \
                 (e.g., 'papers/energy' suggests the file is an energy-related paper).\n\
                 Folder context: {fc}"
            ));
        }
    }

    if !schema.is_empty() {
        parts.push(format!(
            "## Project Schema (page types available — map source content to schema-defined types when it fits)\n{schema}"
        ));
    }
    if !purpose.is_empty() {
        parts.push(format!("## Wiki Purpose (for context)\n{purpose}"));
    }
    if !index.is_empty() {
        parts.push(format!("## Current Wiki Index (for checking existing content)\n{index}"));
    }

    parts.join("\n")
}

// ── Step 2: Generation prompt ───────────────────────────────────

fn build_generation_prompt(
    schema: &str,
    purpose: &str,
    index: &str,
    overview: &str,
    source_file_name: &str,
    source_content: &str,
    source_summary_path: &str,
) -> String {
    let source_base = source_file_name
        .rsplitn(2, '.')
        .last()
        .unwrap_or(source_file_name);
    let summary_path = if source_summary_path.is_empty() {
        format!("wiki/sources/{source_base}.md")
    } else {
        source_summary_path.to_string()
    };
    let today = wiki_today();
    let types_str = GENERATION_WIKI_TYPES.join(" | ");

    let mut parts: Vec<String> = vec![
        "You are a wiki maintainer. Based on the analysis provided, generate wiki files.".into(),
        "Do not output chain-of-thought, hidden reasoning, or explanatory preamble. Reason internally and output only the requested FILE/REVIEW blocks.".into(),
        String::new(),
        language_directive(source_content),
        String::new(),
        format!("## IMPORTANT: Source File"),
        format!("The original source file is: **{source_file_name}**"),
        format!("All wiki pages generated from this source MUST include this filename in their frontmatter `sources` field."),
        format!("Today's date is **{today}**. Use this exact date for all new `created`, `updated`, and wiki/log.md ingest dates."),
        String::new(),
    ];

    if !schema.is_empty() {
        parts.push(format!(
            "## Project Schema and Routing (AUTHORITATIVE)\n\
             {schema}\n\n\
             Use this schema as the primary routing rule for page types and directories.\n\
             If it defines custom folders or distinctions, write pages into those \
             schema-defined folders instead of forcing them into wiki/entities/ or wiki/concepts/.\n\
             Every generated page's frontmatter type must match the schema directory used in its FILE path."
        ));
        parts.push(String::new());
    }

    parts.extend_from_slice(&[
        "## What to generate".into(),
        String::new(),
        format!("1. A source summary page at **{summary_path}** (MUST use this exact path)"),
        "2. Entity or schema-defined typed pages for key named things identified in the analysis.".into(),
        "3. Concept or schema-defined typed pages for key ideas, methods, techniques, and abstractions.".into(),
        "4. An updated wiki/index.md — add new entries to existing categories, preserve all existing entries".into(),
        "5. A log entry for wiki/log.md (just the new entry to append, format: ## [YYYY-MM-DD] ingest | Title)".into(),
        "6. An updated wiki/overview.md — a high-level summary of what the entire wiki covers, updated to reflect the newly ingested source.".into(),
        String::new(),
        "## Frontmatter Rules (CRITICAL — parser is strict)".into(),
        String::new(),
        "Every page begins with a YAML frontmatter block.".into(),
        "1. The VERY FIRST line of the file MUST be exactly `---` (three hyphens, nothing else).".into(),
        "2. Each frontmatter line is a `key: value` pair on its own line.".into(),
        "3. The frontmatter ends with another `---` line on its own.".into(),
        "4. Arrays use the standard YAML inline form `[a, b, c]`.".into(),
        String::new(),
        "Required fields and types:".into(),
        format!("  • type     — one of the known types ({types_str}), or a custom type explicitly defined by the project schema"),
        "  • title    — string (quote it if it contains a colon)".into(),
        format!("  • created  — {today} for new pages (YYYY-MM-DD, no quotes)"),
        format!("  • updated  — {today} for new pages (same as created)"),
        "  • tags     — array of bare strings: `tags: [microbiology, ai]`".into(),
        "  • related  — array of bare wiki page slugs: `related: [foo, bar-baz]`".into(),
        format!("  • sources  — array of source filenames; MUST include \"{source_file_name}\""),
        String::new(),
        "Use [[wikilink]] syntax in the BODY for cross-references between pages.".into(),
        "Use kebab-case filenames.".into(),
        String::new(),
        "## Output Format (MUST FOLLOW EXACTLY — this is how the parser reads your response)".into(),
        String::new(),
        "Your ENTIRE response consists of FILE blocks followed by optional REVIEW blocks. Nothing else.".into(),
        String::new(),
        "FILE block template:".into(),
        "```".into(),
        "---FILE: wiki/path/to/page.md---".into(),
        "(complete file content with YAML frontmatter)".into(),
        "---END FILE---".into(),
        "```".into(),
        String::new(),
        "The FIRST character of your response MUST be `-` (the opening of `---FILE:`).".into(),
        "DO NOT output any preamble, introductory prose, markdown tables, bullet lists, or headings outside of FILE blocks.".into(),
        "DO NOT output any trailing commentary after the last `---END FILE---`.".into(),
        String::new(),
    ]);

    if !purpose.is_empty() {
        parts.push(format!("## Wiki Purpose\n{purpose}"));
    }
    if !index.is_empty() {
        parts.push(format!("## Current Wiki Index (preserve all existing entries, add new ones)\n{index}"));
    }
    if !overview.is_empty() {
        parts.push(format!("## Current Overview (update this to reflect the new source)\n{overview}"));
    }

    parts.push(String::new());
    parts.push(language_directive(source_content));

    parts.join("\n")
}

// ── FILE block parser ────────────────────────────────────────────

#[derive(Debug)]
pub struct ParsedBlock {
    pub path: String,
    pub content: String,
}

pub fn parse_file_blocks(text: &str) -> (Vec<ParsedBlock>, Vec<String>) {
    let normalized = text.replace("\r\n", "\n");
    let lines: Vec<&str> = normalized.lines().collect();
    let mut blocks = Vec::new();
    let mut warnings = Vec::new();
    let mut i = 0;

    while i < lines.len() {
        // Match "---FILE: path---" (case-insensitive)
        let opener = lines[i];
        let path = if let Some(rest) = opener
            .to_uppercase()
            .strip_prefix("---FILE:")
            .and_then(|r| r.strip_suffix("---"))
        {
            rest.trim().to_string()
        } else {
            i += 1;
            continue;
        };
        i += 1;

        let mut content_lines: Vec<&str> = Vec::new();
        let mut fence_marker: Option<char> = None;
        let mut fence_len = 0;
        let mut closed = false;

        while i < lines.len() {
            let line = lines[i];

            // Detect code fences (``` or ~~~)
            if let Some(fence_match) = detect_fence(line) {
                let (ch, len) = fence_match;
                if fence_marker.is_none() {
                    fence_marker = Some(ch);
                    fence_len = len;
                } else if ch == fence_marker.unwrap() && len >= fence_len {
                    fence_marker = None;
                    fence_len = 0;
                }
                content_lines.push(line);
                i += 1;
                continue;
            }

            // Closer outside fence
            if fence_marker.is_none()
                && line.to_uppercase().trim() == "---END FILE---"
            {
                closed = true;
                i += 1;
                break;
            }

            content_lines.push(line);
            i += 1;
        }

        if !closed {
            warnings.push(format!(
                "FILE block \"{path}\" was not closed before end of stream — dropped"
            ));
            continue;
        }
        if path.is_empty() {
            warnings.push("FILE block with empty path skipped".into());
            continue;
        }
        if !is_safe_ingest_path(&path) {
            warnings.push(format!(
                "FILE block with unsafe path \"{path}\" rejected"
            ));
            continue;
        }

        blocks.push(ParsedBlock {
            path,
            content: content_lines.join("\n"),
        });
    }

    (blocks, warnings)
}

/// Detect a CommonMark code-fence line. Returns Some((char, len)) or None.
fn detect_fence(line: &str) -> Option<(char, usize)> {
    let trimmed = line.trim_start();
    let indent = line.len() - trimmed.len();
    if indent > 3 {
        return None;
    }
    let ch = trimmed.chars().next()?;
    if ch != '`' && ch != '~' {
        return None;
    }
    let len = trimmed.chars().take_while(|c| *c == ch).count();
    if len >= 3 {
        Some((ch, len))
    } else {
        None
    }
}

fn is_safe_ingest_path(p: &str) -> bool {
    if p.trim().is_empty() {
        return false;
    }
    if p.contains('\x00') || p.chars().any(|c| (c as u32) < 0x20) {
        return false;
    }
    if p.starts_with('/') || p.starts_with('\\') {
        return false;
    }
    // Windows drive letter
    if p.len() >= 2 && p.as_bytes()[1] == b':' {
        return false;
    }
    let normalized = p.replace('\\', "/");
    if normalized.split('/').any(|seg| seg == "..") {
        return false;
    }
    if !normalized.starts_with("wiki/") {
        return false;
    }
    true
}

// ── file content reading ─────────────────────────────────────────

/// Read and preprocess a source file.  For PDF / DOCX / PPTX / XLSX
/// we delegate to the same `preprocess_file` path the Tauri command
/// uses.  Plain text is read directly.
fn read_source_content(project_path: &str, rel_path: &str) -> Result<String, String> {
    let abs = Path::new(project_path).join(rel_path);
    let ext = abs
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();

    // Binary formats: use the existing preprocessing
    let office_exts = ["pdf", "docx", "pptx", "xlsx", "xls", "ods"];
    if office_exts.contains(&ext.as_str()) {
        let text = crate::commands::fs::preprocess_file_sync(
            &abs.to_string_lossy(),
        )?;
        if text != "no preprocessing needed" {
            return Ok(text);
        }
        // Fall through: preprocess_file_sync returned "no preprocessing needed"
        // which means we should try reading as plain text (e.g. .doc is
        // listed in OFFICE_EXTS but office_oxide may not handle legacy .doc).
    }

    // Plain text
    fs::read_to_string(&abs).map_err(|e| format!("Cannot read {rel_path}: {e}"))
}

// ── LLM helpers ──────────────────────────────────────────────────

/// Call the LLM and collect the full response text (non-streaming).
async fn llm_collect(
    config: &ChatLlmConfig,
    system_prompt: &str,
    user_prompt: &str,
) -> Result<String, String> {
    let provider = chat::build_provider_config_for_ingest(config, system_prompt, user_prompt)?;
    eprintln!("  calling LLM: {} {} ({})", config.provider, config.model, provider.url);
    let (answer, _events) = chat::stream_llm_raw(&provider).await?;
    Ok(answer)
}

// ── main ingest entry point ──────────────────────────────────────

pub struct IngestResult {
    pub files_written: Vec<String>,
    pub cache_hit: bool,
    pub warnings: Vec<String>,
}

/// Run the two-step ingest pipeline for a single source file.
pub async fn run_ingest(
    project_path: &str,
    source_rel: &str,
    config: &ChatLlmConfig,
    force: bool,
    folder_context: Option<&str>,
) -> Result<IngestResult, String> {
    let pp = Path::new(project_path);

    // 1. Source identity
    let identity = source_identity(source_rel);

    // 2. Read source content
    let content = read_source_content(project_path, source_rel)?;

    // 3. SHA-256 cache check
    let hash = sha256_hex(&content);
    let mut cache = load_cache(project_path);
    if !force {
        if let Some(entry) = cache.entries.get(&identity) {
            if entry.hash == hash {
                return Ok(IngestResult {
                    files_written: vec![],
                    cache_hit: true,
                    warnings: vec![],
                });
            }
        }
    }

    // 4. Load project context
    let purpose = read_if_exists(&pp.join("purpose.md"));
    let schema = read_if_exists(&pp.join("schema.md"));
    let index = read_if_exists(&pp.join("wiki/index.md"));
    let overview = read_if_exists(&pp.join("wiki/overview.md"));
    let file_name = Path::new(source_rel)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or(source_rel);
    let summary_slug = source_summary_slug(&identity);
    let summary_path = format!("wiki/sources/{summary_slug}.md");

    // 5. Step 1: Analysis
    eprintln!("  step 1/2: analyzing ({} chars)...", content.len());
    let analysis_prompt = build_analysis_prompt(
        &purpose, &index, &schema, &content, folder_context,
    );
    let t0 = std::time::Instant::now();
    let analysis = llm_collect(
        config,
        &analysis_prompt,
        &format!("Source file: {file_name}\n\n{content}"),
    )
    .await?;
    eprintln!("  analysis done ({:.1}s, {} chars)", t0.elapsed().as_secs_f64(), analysis.len());

    // 6. Step 2: Generation
    eprintln!("  step 2/2: generating...");
    let generation_prompt = build_generation_prompt(
        &schema, &purpose, &index, &overview, file_name, &content, &summary_path,
    );
    let gen_input = format!(
        "## Analysis of source\n\n{analysis}\n\n\
         ## Source content\n\n{content}",
    );
    let t0 = std::time::Instant::now();
    let generation = llm_collect(config, &generation_prompt, &gen_input).await?;
    eprintln!("  generation done ({:.1}s, {} chars)", t0.elapsed().as_secs_f64(), generation.len());

    // 7. Parse FILE blocks
    let (blocks, warnings) = parse_file_blocks(&generation);

    // 8. Write files
    let wiki_dir = pp.join("wiki");
    let _ = fs::create_dir_all(&wiki_dir);
    let mut files_written: Vec<String> = Vec::new();

    for block in &blocks {
        let dest = pp.join(&block.path);
        if let Some(parent) = dest.parent() {
            let _ = fs::create_dir_all(parent);
        }
        if let Err(e) = fs::write(&dest, &block.content) {
            eprintln!("[ingest] Failed to write {}: {e}", block.path);
        } else {
            files_written.push(block.path.clone());
        }
    }

    // 9. Update cache
    cache.entries.insert(
        identity,
        CacheEntry {
            hash,
            _timestamp: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs(),
            _files_written: files_written.clone(),
        },
    );
    save_cache(project_path, &cache);

    Ok(IngestResult {
        files_written,
        cache_hit: false,
        warnings,
    })
}

fn read_if_exists(path: &Path) -> String {
    fs::read_to_string(path).unwrap_or_default()
}

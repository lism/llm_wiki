//! Standalone wiki ingest CLI.
//!
//! Scans a project's `raw/sources/` directory and rebuilds the wiki
//! by running the two-step LLM ingest pipeline for each file.
//!
//! ```bash
//! cargo build --release --bin llm-wiki-ingest
//!
//! llm-wiki-ingest --project ~/Documents/my-wiki
//! llm-wiki-ingest --project . --file raw/sources/doc.pdf --force
//! llm-wiki-ingest --project . --dry-run
//! ```

use std::path::{Path, PathBuf};

use llm_wiki_lib::api_context::ApiContext;
use llm_wiki_lib::commands::chat::ChatLlmConfig;
use llm_wiki_lib::commands::ingest;
use serde_json;
use uuid::Uuid;

// ── CLI ──────────────────────────────────────────────────────────

struct Args {
    project: PathBuf,
    data_dir: Option<PathBuf>,
    file: Option<String>,
    force: bool,
    dry_run: bool,
    verbose: bool,
    folder_context: Option<String>,
}

fn parse_args() -> Args {
    let raw: Vec<String> = std::env::args().collect();
    let mut args = Args {
        project: PathBuf::from("."),
        data_dir: None,
        file: None,
        force: false,
        dry_run: false,
        verbose: false,
        folder_context: None,
    };

    let mut i = 1;
    while i < raw.len() {
        match raw[i].as_str() {
            "--project" => {
                i += 1;
                args.project = PathBuf::from(&raw[i]);
            }
            "--data-dir" => {
                i += 1;
                args.data_dir = Some(PathBuf::from(&raw[i]));
            }
            "--file" => {
                i += 1;
                args.file = Some(raw[i].clone());
            }
            "--folder-context" => {
                i += 1;
                args.folder_context = Some(raw[i].clone());
            }
            "--force" => args.force = true,
            "--dry-run" => args.dry_run = true,
            "--verbose" | "-v" => args.verbose = true,
            "--help" | "-h" => {
                print_help();
                std::process::exit(0);
            }
            other => {
                eprintln!("Unknown flag: {other}");
                print_help();
                std::process::exit(1);
            }
        }
        i += 1;
    }
    args
}

fn print_help() {
    eprintln!(
        "\
Usage: llm-wiki-ingest [OPTIONS] --project <PATH>

A headless wiki builder.  Reads files from raw/sources/, runs the
two-step LLM ingest pipeline, and writes wiki pages to wiki/.

Options:
  --project <PATH>       Wiki project directory (contains purpose.md,
                         raw/sources/, wiki/)
  --data-dir <PATH>      Config directory with app-state.json for LLM
                         settings (default: platform app-data dir)
  --file <PATH>          Ingest a single file instead of scanning the
                         whole raw/sources/ directory
  --folder-context <STR> Hint for the LLM about categorization
                         (e.g. \"papers/energy\")
  --force                Re-ingest even if SHA-256 cache is unchanged
  --dry-run              Show what would be done without calling LLMs
  --verbose, -v          Print prompts and responses

Environment:
  LLM_WIKI_DATA_DIR      Fallback for --data-dir
  LLM_WIKI_API_TOKEN     LLM API key fallback
"
    );
}

// ── main ─────────────────────────────────────────────────────────

fn main() {
    let args = parse_args();

    if !args.project.exists() {
        eprintln!("Project directory does not exist: {}", args.project.display());
        std::process::exit(1);
    }

    // Resolve data dir
    let data_dir = args.data_dir.unwrap_or_else(|| {
        if let Ok(dir) = std::env::var("LLM_WIKI_DATA_DIR") {
            return PathBuf::from(dir);
        }
        default_data_dir()
    });

    // Load API context
    let rt = tokio::runtime::Runtime::new().expect("Failed to create tokio runtime");
    let handle = rt.handle().clone();
    let ctx = ApiContext::new(data_dir, None, handle);

    // Load LLM config
    let app_state = ctx.load_app_state().unwrap_or_default();
    let Some(llm_config) = llm_wiki_lib::commands::chat::resolve_llm_config(&app_state) else {
        eprintln!("No LLM config found in {}.", ctx.data_dir.display());
        eprintln!("Configure an LLM provider in the desktop app Settings first, or");
        eprintln!("create {}/app-state.json with llmConfig.", ctx.data_dir.display());
        std::process::exit(1);
    };

    let project_path = args.project.canonicalize().unwrap_or_else(|_| args.project.clone());
    let project_str = project_path.to_string_lossy().to_string();

    // Ensure project scaffolding exists (schema.md, purpose.md, wiki/, ...)
    ensure_project_scaffold(&project_path);
    // Register so the API server can discover it
    register_project(&project_path, &ctx.data_dir);

    eprintln!("Project:    {}", project_path.display());
    eprintln!("Data dir:   {}", ctx.data_dir.display());
    eprintln!("Provider:   {} / {}", llm_config.provider, llm_config.model);
    eprintln!();

    // Run ingest
    rt.block_on(async {
        if let Some(ref file) = args.file {
            // Single file mode
            run_single(
                &project_str,
                file,
                &llm_config,
                args.force,
                args.dry_run,
                args.verbose,
                args.folder_context.as_deref(),
            )
            .await;
        } else {
            // Scan raw/sources/
            run_scan(
                &project_str,
                &llm_config,
                args.force,
                args.dry_run,
                args.verbose,
                args.folder_context.as_deref(),
            )
            .await;
        }
    });
}

// ── scan mode ────────────────────────────────────────────────────

async fn run_scan(
    project: &str,
    config: &ChatLlmConfig,
    force: bool,
    dry_run: bool,
    verbose: bool,
    folder_context: Option<&str>,
) {
    let sources_dir = Path::new(project).join("raw/sources");
    if !sources_dir.exists() {
        eprintln!("No raw/sources/ directory found in project.");
        return;
    }

    let mut files: Vec<PathBuf> = Vec::new();
    for entry in walkdir::WalkDir::new(&sources_dir)
        .into_iter()
        .filter_entry(|e| !e.file_name().to_string_lossy().starts_with('.'))
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
    {
        let rel = entry
            .path()
            .strip_prefix(project)
            .unwrap_or(entry.path());
        files.push(rel.to_path_buf());
    }
    files.sort();

    let total = files.len();
    eprintln!("Found {total} files in raw/sources/\n");

    let mut ingested = 0usize;
    let mut skipped = 0usize;
    let mut errors = 0usize;

    for (idx, file) in files.iter().enumerate() {
        let rel = file.to_string_lossy();
        let abs = Path::new(project).join(file);
        let size = abs.metadata().map(|m| m.len()).unwrap_or(0);
        let size_kb = size as f64 / 1024.0;

        eprint!("[{}/{}] {rel}  ({size_kb:.0} KB)  ", idx + 1, total);

        if dry_run {
            eprintln!("dry-run: would ingest");
            continue;
        }

        let t0 = std::time::Instant::now();
        match ingest::run_ingest(project, &rel, config, force, folder_context).await {
            Ok(result) => {
                let elapsed = t0.elapsed().as_secs_f64();
                if result.cache_hit {
                    skipped += 1;
                    eprintln!("cache hit → skipped  ({elapsed:.1}s)");
                } else {
                    ingested += 1;
                    let n = result.files_written.len();
                    eprintln!("→ {n} pages  ({elapsed:.1}s)");
                    if verbose {
                        for f in &result.files_written {
                            eprintln!("  wrote: {f}");
                        }
                    }
                    for w in &result.warnings {
                        eprintln!("  warning: {w}");
                    }
                }
            }
            Err(e) => {
                errors += 1;
                let elapsed = t0.elapsed().as_secs_f64();
                eprintln!("ERROR: {e}  ({elapsed:.1}s)");
                if verbose {
                    eprintln!("  {e}");
                }
            }
        }
    }

    eprintln!();
    eprintln!(
        "Done.  {total} files: {ingested} ingested, {skipped} cached, {errors} errors."
    );
}

// ── single-file mode ─────────────────────────────────────────────

async fn run_single(
    project: &str,
    file: &str,
    config: &ChatLlmConfig,
    force: bool,
    dry_run: bool,
    verbose: bool,
    folder_context: Option<&str>,
) {
    if dry_run {
        eprintln!("dry-run: would ingest {file}");
        return;
    }

    let t0 = std::time::Instant::now();
    match ingest::run_ingest(project, file, config, force, folder_context).await {
        Ok(result) => {
            let elapsed = t0.elapsed().as_secs_f64();
            if result.cache_hit {
                eprintln!("cache hit → skipped  ({elapsed:.1}s)");
                return;
            }
            eprintln!("Done.  {} pages written  ({elapsed:.1}s)", result.files_written.len());
            for f in &result.files_written {
                eprintln!("  {f}");
            }
            for w in &result.warnings {
                eprintln!("  warning: {w}");
            }
        }
        Err(e) => {
            let elapsed = t0.elapsed().as_secs_f64();
            eprintln!("ERROR  ({elapsed:.1}s)");
            if verbose {
                eprintln!("{e}");
            } else {
                let short: String = e.chars().take(200).collect();
                eprintln!("{short}");
            }
        }
    }
}

// ── platform helpers ─────────────────────────────────────────────

/// Create project scaffolding files (schema.md, purpose.md, wiki/…) if
/// they don't already exist.
fn ensure_project_scaffold(project_path: &Path) {
    let schema = project_path.join("schema.md");
    if !schema.exists() {
        let content = include_str!("../../commands/project_schema_template.md");
        let _ = std::fs::write(&schema, content);
        eprintln!("Created schema.md");
    }

    let purpose = project_path.join("purpose.md");
    if !purpose.exists() {
        let content = include_str!("../../commands/project_purpose_template.md");
        let _ = std::fs::write(&purpose, content);
        eprintln!("Created purpose.md");
    }

    let wiki = project_path.join("wiki");
    let _ = std::fs::create_dir_all(&wiki);

    let index = wiki.join("index.md");
    if !index.exists() {
        let _ = std::fs::write(&index, "# Wiki Index\n\n");
    }

    let log = wiki.join("log.md");
    if !log.exists() {
        let _ = std::fs::write(&log, "# Wiki Log\n\n");
    }

    let overview = wiki.join("overview.md");
    if !overview.exists() {
        let _ = std::fs::write(&overview, "# Wiki Overview\n\n");
    }

    let raw = project_path.join("raw/sources");
    let _ = std::fs::create_dir_all(&raw);
}

/// Create `.llm-wiki/project.json` and register the project in
/// `app-state.json` so the API server can discover it.
fn register_project(project_path: &Path, data_dir: &Path) {
    let llm_wiki_dir = project_path.join(".llm-wiki");
    let _ = std::fs::create_dir_all(&llm_wiki_dir);

    let project_json = llm_wiki_dir.join("project.json");
    let project_id = if project_json.exists() {
        // Read existing ID
        std::fs::read_to_string(&project_json)
            .ok()
            .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
            .and_then(|v| v.get("id").and_then(|id| id.as_str()).map(String::from))
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string())
    } else {
        let id = uuid::Uuid::new_v4().to_string();
        let json = serde_json::json!({
            "id": &id,
            "name": project_path.file_name().unwrap_or_default().to_string_lossy(),
        });
        let _ = std::fs::write(&project_json, serde_json::to_string_pretty(&json).unwrap_or_default());
        id
    };

    // Register in app-state.json
    let app_state_path = data_dir.join("app-state.json");
    let mut state: serde_json::Value = std::fs::read_to_string(&app_state_path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or(serde_json::json!({}));

    let proj_name = project_path.file_name().unwrap_or_default().to_string_lossy().to_string();
    let proj_path = project_path.to_string_lossy().to_string();

    // Update projectRegistry
    if let Some(registry) = state.get_mut("projectRegistry") {
        if let Some(obj) = registry.as_object_mut() {
            obj.entry(&project_id).or_insert_with(|| serde_json::json!({
                "name": &proj_name,
                "path": &proj_path,
            }));
        }
    } else {
        state["projectRegistry"] = serde_json::json!({
            &project_id: { "name": &proj_name, "path": &proj_path }
        });
    }

    // Update recentProjects
    if let Some(recents) = state.get_mut("recentProjects") {
        if let Some(arr) = recents.as_array_mut() {
            let exists = arr.iter().any(|v| {
                v.get("path").and_then(|p| p.as_str()) == Some(&proj_path)
            });
            if !exists {
                arr.push(serde_json::json!({ "name": &proj_name, "path": &proj_path }));
            }
        }
    } else {
        state["recentProjects"] = serde_json::json!([{ "name": &proj_name, "path": &proj_path }]);
    }

    if let Ok(json) = serde_json::to_string_pretty(&state) {
        let _ = std::fs::write(&app_state_path, json);
    }

    eprintln!("Registered project {} ({})", proj_name, project_id);
}

fn default_data_dir() -> PathBuf {
    #[cfg(target_os = "macos")]
    {
        let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
        PathBuf::from(home).join("Library/Application Support/com.llmwiki.app")
    }
    #[cfg(target_os = "linux")]
    {
        let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
        PathBuf::from(home).join(".local/share/com.llmwiki.app")
    }
    #[cfg(target_os = "windows")]
    {
        std::env::var("APPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("."))
            .join("com.llmwiki.app")
    }
}

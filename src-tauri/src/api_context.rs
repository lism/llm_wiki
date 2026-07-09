//! Shared context for the local HTTP API server.
//!
//! Replaces `AppHandle` as the single dependency for `api_server.rs`,
//! so the same server logic can be used from both the Tauri desktop
//! app and a future standalone binary.
//!
//! The struct holds everything the API routes need: a data directory
//! (for `app-state.json`), an optional resource directory (for PDFium
//! binaries), the project registry, and the current project path.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::commands::search::SearchEmbeddingConfig;

/// All state the API server needs to serve requests.
pub struct ApiContext {
    /// Directory containing `app-state.json` (and `.llm-wiki/…`
    /// project state).  In the Tauri app this is
    /// `app.path().app_data_dir()`; standalone users pass `--data-dir`.
    pub data_dir: PathBuf,

    /// Directory containing PDFium binaries (`libpdfium.so`,
    /// `pdfium.dll`, …).  Optional — when `None` the PDF extractor
    /// won't be initialised and `FileNode::no_pdf_extractor` will be
    /// `true`, same as when the resource dir hint is missing today.
    pub resource_dir: Option<PathBuf>,

    /// Project registry — the in-memory list of known wiki projects.
    /// Populated once at startup and kept in sync by the clip server
    /// (Tauri) or loaded from `app-state.json` (standalone).
    pub projects: Mutex<Vec<ProjectEntry>>,

    /// Absolute path of the "current" project (last opened in the
    /// desktop app).  Used by the clip server and the `projects`
    /// API endpoint.
    pub current_project: Mutex<Option<String>>,

    /// Handle to a running tokio runtime, used to bridge async
    /// commands (search, chat, vector store) into synchronous HTTP
    /// request threads via `block_on`.
    pub tokio_handle: tokio::runtime::Handle,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectEntry {
    pub id: String,
    pub name: String,
    pub path: String,
    pub current: bool,
}

impl ApiContext {
    /// Build an `ApiContext` from a data directory, an optional
    /// resource directory, and a tokio runtime handle.  Loads the
    /// project registry from `app-state.json` if it exists.
    pub fn new(
        data_dir: PathBuf,
        resource_dir: Option<PathBuf>,
        tokio_handle: tokio::runtime::Handle,
    ) -> Self {
        let projects = load_projects_from_disk(&data_dir);
        let current = projects.iter().find(|p| p.current).map(|p| p.path.clone());
        Self {
            data_dir,
            resource_dir,
            projects: Mutex::new(projects),
            current_project: Mutex::new(current),
            tokio_handle,
        }
    }

    /// Read `app-state.json` from the data directory.
    pub fn load_app_state(&self) -> Option<Value> {
        let path = self.data_dir.join("app-state.json");
        let raw = fs::read_to_string(path).ok()?;
        serde_json::from_str::<Value>(&raw).ok()
    }

    /// Load the embedding config from app-state.json.
    pub fn load_embedding_config(&self) -> Option<SearchEmbeddingConfig> {
        let parsed = self.load_app_state()?;
        let value = parsed.get("embeddingConfig")?.clone();
        serde_json::from_value::<SearchEmbeddingConfig>(value).ok()
    }

    /// Whether the API is enabled (reads `apiConfig.enabled`).
    pub fn api_enabled(&self) -> bool {
        self.load_app_state()
            .and_then(|v| v.get("apiConfig")?.get("enabled")?.as_bool())
            .unwrap_or(true)
    }

    /// Whether MCP mode is enabled.
    pub fn api_mcp_enabled(&self) -> bool {
        self.load_app_state()
            .and_then(|v| v.get("apiConfig")?.get("mcpEnabled")?.as_bool())
            .unwrap_or(false)
    }

    /// Whether unauthenticated local access is allowed.
    pub fn api_allow_unauthenticated(&self) -> bool {
        self.load_app_state()
            .and_then(|v| v.get("apiConfig")?.get("allowUnauthenticated")?.as_bool())
            .unwrap_or(false)
    }

    /// Whether LAN access is allowed (binds to 0.0.0.0).
    pub fn api_allow_lan_access(&self) -> bool {
        self.load_app_state()
            .and_then(|v| v.get("apiConfig")?.get("allowLanAccess")?.as_bool())
            .unwrap_or(false)
    }

    /// The API token from `app-state.json` or the
    /// `LLM_WIKI_API_TOKEN` environment variable.
    pub fn api_token(&self) -> Option<String> {
        if let Ok(token) = std::env::var("LLM_WIKI_API_TOKEN") {
            let trimmed = token.trim();
            if !trimmed.is_empty() {
                return Some(trimmed.to_string());
            }
        }
        let parsed = self.load_app_state()?;
        parsed
            .get("apiConfig")
            .and_then(|v| v.get("token"))
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(ToOwned::to_owned)
    }

    /// Where the token was sourced from.
    pub fn api_token_source(&self) -> &'static str {
        if let Ok(token) = std::env::var("LLM_WIKI_API_TOKEN") {
            if !token.trim().is_empty() {
                return "env";
            }
        }
        if self.api_token().is_some() {
            "store"
        } else {
            "none"
        }
    }

    /// The effective bind host (env var → store → default).
    pub fn configured_bind_host(&self) -> String {
        if let Ok(host) = std::env::var("LLM_WIKI_BIND_HOST") {
            let trimmed = host.trim();
            if !trimmed.is_empty() {
                return trimmed.to_string();
            }
        }
        if self.api_allow_lan_access() {
            "0.0.0.0".to_string()
        } else {
            "127.0.0.1".to_string()
        }
    }
}

// ── project registry persistence ─────────────────────────────────

fn load_projects_from_disk(data_dir: &Path) -> Vec<ProjectEntry> {
    let path = data_dir.join("app-state.json");
    let raw = match fs::read_to_string(&path) {
        Ok(s) => s,
        Err(_) => return Vec::new(),
    };
    let parsed: Value = match serde_json::from_str(&raw) {
        Ok(v) => v,
        Err(_) => return Vec::new(),
    };

    let mut by_path: std::collections::BTreeMap<String, ProjectEntry> =
        std::collections::BTreeMap::new();

    // Read from projectRegistry
    if let Some(registry) = parsed.get("projectRegistry").and_then(|v| v.as_object()) {
        for (id, value) in registry {
            let path = value.get("path").and_then(|v| v.as_str()).unwrap_or("");
            if path.is_empty() {
                continue;
            }
            let path = normalize_path(path);
            let name = value
                .get("name")
                .and_then(|v| v.as_str())
                .map(ToOwned::to_owned)
                .unwrap_or_else(|| project_name_from_path(&path));
            by_path.entry(path.clone()).or_insert(ProjectEntry {
                id: id.clone(),
                name,
                path,
                current: false,
            });
        }
    }

    // Read from recentProjects
    if let Some(recents) = parsed.get("recentProjects").and_then(|v| v.as_array()) {
        for value in recents {
            let path = value.get("path").and_then(|v| v.as_str()).unwrap_or("");
            if path.is_empty() {
                continue;
            }
            let path = normalize_path(path);
            by_path.entry(path.clone()).or_insert_with(|| {
                let id = read_project_id(&path).unwrap_or_else(|| path.clone());
                let name = value
                    .get("name")
                    .and_then(|v| v.as_str())
                    .map(ToOwned::to_owned)
                    .unwrap_or_else(|| project_name_from_path(&path));
                ProjectEntry {
                    id,
                    name,
                    path,
                    current: false,
                }
            });
        }
    }

    by_path.into_values().collect()
}

fn read_project_id(path: &str) -> Option<String> {
    let raw = fs::read_to_string(Path::new(path).join(".llm-wiki/project.json")).ok()?;
    let parsed: Value = serde_json::from_str(&raw).ok()?;
    parsed.get("id").and_then(Value::as_str).map(ToOwned::to_owned)
}

fn project_name_from_path(path: &str) -> String {
    Path::new(path)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("Project")
        .to_string()
}

fn normalize_path(path: &str) -> String {
    path.replace('\\', "/").trim_end_matches('/').to_string()
}

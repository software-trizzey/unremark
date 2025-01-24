use serde::{Serialize, Deserialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::fs;

#[derive(Debug, Clone, Copy)]
pub enum Language {
    Python,
    JavaScript,
    TypeScript,
    Rust,
}

impl Language {
    pub fn from_extension(ext: &str) -> Option<Self> {
        match ext {
            "py" => Some(Language::Python),
            "js" => Some(Language::JavaScript),
            "ts" => Some(Language::TypeScript),
            "rs" => Some(Language::Rust),
            _ => None,
        }
    }

    pub fn get_tree_sitter_language(&self) -> tree_sitter::Language {
        match self {
            Language::Python => tree_sitter_python::LANGUAGE.into(),
            Language::JavaScript => tree_sitter_javascript::LANGUAGE.into(),
            Language::TypeScript => tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
            Language::Rust => tree_sitter_rust::LANGUAGE.into(),
        }
    }
}

#[derive(Debug)]
pub struct AnalysisResult {
    pub path: PathBuf,
    pub redundant_comments: Vec<CommentInfo>,
    pub errors: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommentInfo {
    pub text: String,
    pub line_number: usize,
    pub context: String,
}

#[derive(Debug, Deserialize)]
pub struct CommentAnalysis {
    pub is_redundant: bool,
    pub comment_line_number: usize,
    pub explanation: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CacheEntry {
    pub last_modified: u64,
    pub redundant_comments: Vec<CommentInfo>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Cache {
    pub entries: HashMap<String, CacheEntry>,
}

impl Cache {
    pub fn load_from_path(cache_path: &PathBuf) -> Self {
        match fs::read_to_string(cache_path) {
            Ok(contents) => serde_json::from_str(&contents).unwrap_or(Cache {
                entries: HashMap::new(),
            }),
            Err(_) => Cache {
                entries: HashMap::new(),
            },
        }
    }

    pub fn save_to_path(&self, cache_path: &PathBuf) {
        if let Ok(contents) = serde_json::to_string(self) {
            let _ = fs::write(cache_path, contents);
        }
    }

    pub fn load() -> Self {
        Self::load_from_path(&crate::get_cache_path())
    }

    pub fn save(&self) {
        self.save_to_path(&crate::get_cache_path())
    }
}

#[derive(Debug)]
pub enum ApiError {
    RateLimit(String),
    Timeout(String),
    Network(String),
    Other(String),
}

impl std::fmt::Display for ApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ApiError::RateLimit(msg) => write!(f, "Rate limit exceeded: {}", msg),
            ApiError::Timeout(msg) => write!(f, "Request timeout: {}", msg),
            ApiError::Network(msg) => write!(f, "Network error: {}", msg),
            ApiError::Other(msg) => write!(f, "API error: {}", msg),
        }
    }
}

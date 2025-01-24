use std::path::PathBuf;
use std::fs;
use log::debug;

// Public exports
pub use crate::types::{
    Language,
    CommentInfo,
    CommentAnalysis,
    AnalysisResult,
    ApiError,
    Cache,
    CacheEntry,
};
pub use crate::analysis::{analyze_file, analyze_comments};
pub use crate::utils::{find_context, remove_redundant_comments};

// Internal modules
mod types;
mod analysis;
mod utils;
mod api;

// Constants
pub const CACHE_FILE_NAME: &str = "analysis_cache.json";

// Public functions
pub fn get_cache_path() -> PathBuf {
    let cache_dir = dirs::cache_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("unremark");
    
    debug!("Cache directory: {}", cache_dir.display());
    fs::create_dir_all(&cache_dir).unwrap_or_default();
    
    cache_dir.join(CACHE_FILE_NAME)
}

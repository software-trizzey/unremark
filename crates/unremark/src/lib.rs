pub use crate::types::{
    Language,
    CommentInfo,
    CommentAnalysis,
    AnalysisResult,
    ApiError,
    Cache,
    CacheEntry,
};
pub use crate::analysis::{analyze_file, analyze_comments, analyze_current_file};
pub use crate::utils::{find_context, remove_redundant_comments};
pub use crate::comment_detection::detect_comments;
pub use crate::constants::{OPENAI_MODEL, CACHE_FILE_NAME, get_proxy_endpoint};
pub use services::proxy::{ProxyAnalysisService, AnalysisService, create_analysis_service};

// Internal modules
mod types;
mod constants;
mod analysis;
mod utils;
mod api;
mod comment_detection;
mod bindings;
mod services;


// Python bindings (only when python feature is enabled)
#[cfg(feature = "python")]
pub use bindings::python::{py_analyze_comments, PyCommentInfo};

#[cfg(feature = "python")]
use pyo3::prelude::*;

#[cfg(feature = "python")]
#[pymodule]
fn unremark(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<PyCommentInfo>()?;
    m.add_function(wrap_pyfunction!(py_analyze_comments, m)?)?;
    Ok(())
}

#[cfg(feature = "python")]
use pyo3::prelude::*;

#[cfg(feature = "python")]
use crate::types::CommentInfo;

#[cfg(feature = "python")]
#[pyclass]
#[derive(Clone)]
pub struct PyCommentInfo {
    #[pyo3(get)]
    text: String,
    #[pyo3(get)]
    line_number: usize,
    #[pyo3(get)]
    context: String,
}

#[cfg(feature = "python")]
#[pymethods]
impl PyCommentInfo {
    #[new]
    fn new(text: String, line_number: usize, context: String) -> Self {
        Self { text, line_number, context }
    }
}

#[cfg(feature = "python")]
#[pyfunction]
pub fn py_analyze_comments(comments: Vec<PyCommentInfo>) -> PyResult<Vec<PyCommentInfo>> {
    let rust_comments = comments.into_iter()
        .map(|c| CommentInfo {
            text: c.text,
            line_number: c.line_number,
            context: c.context,
        })
        .collect();

    let redundant_comments = tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(crate::analysis::analyze_comments(rust_comments))
        .map_err(|e| PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(e.to_string()))?;

    Ok(redundant_comments.into_iter()
        .map(|c| PyCommentInfo {
            text: c.text,
            line_number: c.line_number,
            context: c.context,
        })
        .collect())
}
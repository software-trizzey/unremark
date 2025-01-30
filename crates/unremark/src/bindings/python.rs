#[cfg(feature = "python")]
use pyo3::prelude::*;

#[cfg(feature = "python")]
use crate::types::Language;

#[cfg(feature = "python")]
use crate::comment_detection::detect_comments;

#[cfg(feature = "python")]
#[pyclass]
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
pub fn py_analyze_comments(source_code: &str, language: &str) -> PyResult<Vec<PyCommentInfo>> {
    let lang = Language::from_extension(language)
        .ok_or_else(|| PyErr::new::<pyo3::exceptions::PyValueError, _>("Unsupported language"))?;
    
    let comments = detect_comments(source_code, lang)
        .map_err(|e| PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(e.to_string()))?;
    
    Ok(comments.into_iter()
        .map(|c| PyCommentInfo {
            text: c.text,
            line_number: c.line_number,
            context: c.context,
        })
        .collect())
}

#[cfg(feature = "python")]
pub fn register_module(py: Python<'_>, m: &PyModule) -> PyResult<()> {
    m.add_class::<PyCommentInfo>()?;
    m.add_function(wrap_pyfunction!(py_analyze_comments, m)?)?;
    Ok(())
} 
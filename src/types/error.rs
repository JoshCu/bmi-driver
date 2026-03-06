use crate::library::DlError;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum BmiError {
    #[error("Failed to load library '{path}': {source}")]
    LibraryLoad {
        path: String,
        #[source]
        source: DlError,
    },

    #[error("Function '{func}' not found: {source}")]
    SymbolNotFound {
        func: String,
        #[source]
        source: DlError,
    },

    #[error("BMI function '{func}' failed for model '{model}'")]
    FunctionFailed { model: String, func: String },

    #[error("Function '{func}' not implemented by model '{model}'")]
    NotImplemented { model: String, func: String },

    #[error("Model '{model}' not initialized")]
    NotInitialized { model: String },

    #[error("Model '{model}' already initialized")]
    AlreadyInitialized { model: String },

    #[error("Config file not found: {path}")]
    ConfigNotFound { path: String },

    #[error("Invalid UTF-8 string")]
    InvalidUtf8,
}

pub type BmiResult<T> = Result<T, BmiError>;

/// Shorthand for creating a `BmiError::FunctionFailed`.
pub fn function_failed(model: impl Into<String>, msg: impl Into<String>) -> BmiError {
    BmiError::FunctionFailed {
        model: model.into(),
        func: msg.into(),
    }
}

/// Shorthand for creating a `BmiError::NotImplemented`.
pub fn not_implemented(model: impl Into<String>, func: impl Into<String>) -> BmiError {
    BmiError::NotImplemented {
        model: model.into(),
        func: func.into(),
    }
}

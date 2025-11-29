//! Error types for BMI operations.

use crate::library::DlError;
use thiserror::Error;

/// Errors that can occur when working with BMI models.
#[derive(Error, Debug)]
pub enum BmiError {
    /// Failed to load the shared library
    #[error("Failed to load library '{path}': {source}")]
    LibraryLoad {
        path: String,
        #[source]
        source: DlError,
    },

    /// Failed to find the registration function in the library
    #[error("Failed to find function '{func}' in library: {source}")]
    RegistrationFunctionNotFound {
        func: String,
        #[source]
        source: DlError,
    },

    /// A BMI function returned a failure code
    #[error("BMI function '{func}' failed for model '{model}'")]
    BmiFunctionFailed { model: String, func: String },

    /// A required BMI function pointer is not set
    #[error("BMI function '{func}' is not implemented by model '{model}'")]
    FunctionNotImplemented { model: String, func: String },

    /// Model has not been initialized
    #[error("Model '{model}' has not been initialized")]
    NotInitialized { model: String },

    /// Model has already been initialized  
    #[error("Model '{model}' has already been initialized")]
    AlreadyInitialized { model: String },

    /// Configuration file not found or not readable
    #[error("Config file not found or unreadable: {path}")]
    ConfigFileNotFound { path: String },

    /// Invalid UTF-8 in string returned from model
    #[error("Invalid UTF-8 string returned from model")]
    InvalidUtf8,
}

/// Result type alias for BMI operations
pub type BmiResult<T> = Result<T, BmiError>;

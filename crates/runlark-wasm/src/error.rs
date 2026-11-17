use thiserror::Error;

/// Errors produced by WIT contract checks, package manifest verification,
/// digest pinning, and bounded Wasmtime validator execution.
#[derive(Debug, Error)]
pub enum WasmError {
    /// Failed to read or write package or CAS files on disk.
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    /// Failed to parse or serialize JSON manifest or workflow definitions.
    #[error("JSON parse error: {0}")]
    Json(#[from] serde_json::Error),

    /// Failed to parse or validate a WIT contract.
    #[error("WIT contract error: {0}")]
    WitContract(String),

    /// Package manifest failed schema, `SemVer`, path, or capability validation.
    #[error("invalid package manifest: {0}")]
    InvalidManifest(String),

    /// Declarative workflow definition failed stage, transition, or budget validation.
    #[error("invalid workflow definition: {0}")]
    InvalidWorkflow(String),

    /// Computed SHA-256 digest does not match the pinned manifest digest.
    #[error("digest mismatch for {artifact}: expected {expected}, actual {actual}")]
    DigestMismatch {
        artifact: String,
        expected: String,
        actual: String,
    },

    /// Requested package or pinned digest is not present in the registry.
    #[error("package digest not found: {0}")]
    PackageNotFound(String),

    /// Failed to assemble WAT or encode/validate a Component Model binary.
    #[error("WASM component build error: {0}")]
    ComponentBuild(String),

    /// WASM component declared disallowed imports or lacked required exports.
    #[error("WASM component compatibility error: {0}")]
    IncompatibleComponent(String),

    /// Wasmtime compilation, instantiation, or bounded execution failed (including out-of-fuel).
    #[error("Wasmtime execution error: {0}")]
    Execution(String),

    /// Guest validator returned an explicit domain error (`Err(string)`).
    #[error("validator rejected input: {0}")]
    ValidatorRejected(String),
}

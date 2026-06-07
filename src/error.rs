use std::path::PathBuf;

#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error("unsupported system backend: {backend}")]
    UnsupportedBackend { backend: String },

    #[error("missing system event source: {name}")]
    MissingEventSource { name: String },

    #[error("niri command failed: {detail}")]
    NiriCommandFailed { detail: String },

    #[error("niri json decode failed: {source}")]
    NiriJson { source: serde_json::Error },

    #[error("target not found: {target:?}")]
    TargetNotFound { target: crate::SystemTarget },

    #[error("actor call failed: {detail}")]
    ActorCall { detail: String },

    #[error("signal frame: {0}")]
    SignalFrame(#[from] signal_core::FrameError),

    #[error("daemon argument: {0}")]
    Argument(#[from] triad_runtime::ArgumentError),

    #[error("missing command-line input")]
    MissingInput,

    #[error("system socket path is missing")]
    MissingSocket,

    #[error("unexpected command-line argument: {got}")]
    UnexpectedArgument { got: String },

    #[error("unexpected signal frame: {got}")]
    UnexpectedSignalFrame { got: String },

    #[error("invalid inline nota argument: {got}")]
    InvalidInlineNotaArgument { got: String },

    #[error("input file read failed at {path}: {source}")]
    InputFileRead {
        path: PathBuf,
        source: std::io::Error,
    },

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("nota error: {0}")]
    Nota(#[from] nota_codec::Error),

    #[error("failed to read system daemon configuration {path:?}: {source}")]
    ConfigurationRead {
        path: PathBuf,
        source: std::io::Error,
    },

    #[error("failed to write system daemon configuration {path:?}: {source}")]
    ConfigurationWrite {
        path: PathBuf,
        source: std::io::Error,
    },

    #[error("failed to decode system daemon configuration archive")]
    ConfigurationArchiveDecode,

    #[error("failed to encode system daemon configuration archive")]
    ConfigurationArchiveEncode,
}

impl From<serde_json::Error> for Error {
    fn from(source: serde_json::Error) -> Self {
        Self::NiriJson { source }
    }
}

pub type Result<T> = std::result::Result<T, Error>;

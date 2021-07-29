use thiserror::Error;

#[derive(Error, Debug)]
pub enum AppError {
    #[error(transparent)]
    Scm(#[from] git2::Error),

    #[error("unexpected object type")]
    UnexpectedObjectType(git2::ObjectType),

    #[error(transparent)]
    FromUtf8Error(#[from] std::string::FromUtf8Error),

    #[error(transparent)]
    FromTryFromIntError(#[from] std::num::TryFromIntError),

    #[error(transparent)]
    AnyhowError(#[from] anyhow::Error),

    #[error("missing object type")]
    MissingObjectType,

    #[error(transparent)]
    Io(#[from] std::io::Error),

    #[error("expected something")]
    None(),

    #[error("expected the object to have a size")]
    Unsized(),

    #[error("failed to acquire reader lock")]
    ReadLockFailed(),

    #[error("failed to acquire writer lock")]
    WriteLockFailed(),

    #[error("missing item")]
    Missing(),

    #[error("repo not enabled")]
    NotEnabled(),

    #[error("unsupported hash digest")]
    UnsupportedDigest(),

    #[error("invalid arguments")]
    InvalidArgs(),

    #[error("argument overridden")]
    ArgumentOverridden(String),

    #[error("current state other than expected")]
    IncorrectState(),

    #[error("not implemented")]
    NotImplemented(),

    #[error(transparent)]
    Var(#[from] std::env::VarError),

    #[error("unable to determine work directory for repository")]
    InvalidWorkDir(),

    #[error("Decoding data failed")]
    DecodeError(#[from] prost::DecodeError),

    #[error("Encoding data failed")]
    EncodeError(#[from] prost::EncodeError),

    #[error(transparent)]
    InvalidUri(#[from] http::uri::InvalidUri),
}

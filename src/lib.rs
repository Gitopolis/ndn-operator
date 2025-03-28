use thiserror::Error;

#[derive(Error, Debug)]
pub enum Error {
    #[error("SerializationError: {0}")]
    SerializationError(#[source] serde_json::Error),

    #[error("Kube Error: {0}")]
    KubeError(#[source] kube::Error),

    #[error("Finalizer Error: {0}")]
    // NB: awkward type because finalizer::Error embeds the reconciler error (which is this)
    // so boxing this error to break cycles
    FinalizerError(#[source] Box<kube::runtime::finalizer::Error<Error>>),
}
pub type Result<T, E = Error> = std::result::Result<T, E>;

pub mod controller;
pub mod daemonset;
pub mod ndnd;
pub use crate::controller::*;
pub use crate::ndnd::*;

/// Log and trace integrations
pub mod telemetry;
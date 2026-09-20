//! velqu-view-starter library surface.
//!
//! The starter is both a binary (the app host: window loop, CLI) and a
//! small library so its integration tests can drive the exact host
//! code the binary uses — the workflow logic ([`ops`]) and the
//! app-owned window loop ([`window`]) — through VelquView's public
//! API, never the framework's internals.

pub mod ops;
pub mod window;

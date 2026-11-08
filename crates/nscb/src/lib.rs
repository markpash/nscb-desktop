// Library API: many public items are not consumed by the binary yet.
#![allow(dead_code)]

pub mod api;
pub mod cli;
pub mod crypto;
pub mod error;
pub mod formats;
pub mod keys;
pub mod nutdb;
pub mod ops;
pub mod util;

/// Version of the vendored nscb_rust engine.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

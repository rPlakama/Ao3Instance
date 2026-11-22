//! AO3Instance — core library.
//!
//! Layer map (see `docs/write-client-plan.md`):
//! `crypto` holds the secret store, `ao3::session` owns the account state machine,
//! `ao3::jar` the cookie jar, `ao3::client` the HTTP boundary.

pub mod ao3;
pub mod config;
pub mod consts;
pub mod crypto;
pub mod error;

//! PipeDeck daemon library.
//!
//! `pipedeckd` owns all PipeWire interaction and republishes it on the session
//! bus as `dev.pipedeck.Daemon1` (SPEC §2.2), so the GNOME Shell extension, the
//! `pipedeck` CLI and any future front end are all thin D-Bus clients.
//!
//! Module layout follows one rule: [`pw`] is the *only* module that links
//! against libpipewire. Everything else — [`config`], [`eq`], [`matching`],
//! [`meta`], [`route`], [`state`], [`volume`] — is pure data and is unit-tested
//! without a graph.

#![warn(missing_docs)]

pub mod command;
pub mod config;
pub mod eq;
pub mod error;
pub mod matching;
pub mod meta;
pub mod pw;
pub mod route;
pub mod service;
pub mod state;
pub mod volume;

/// Version reported by the `Version` D-Bus property and `--version`.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

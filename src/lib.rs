//! Stratum — a native Git GUI client for Linux.
//!
//! The library crate contains everything except the GTK user interface so
//! that the Git service layer and application core can be tested headlessly.

pub mod core;
pub mod git;
pub mod ui;

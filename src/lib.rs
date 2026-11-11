//! Rust library surface for the canonical kamishibai runtime.

#![forbid(unsafe_code)]

pub mod anki;
pub mod application;
pub mod cli;
pub mod config;
pub mod gemini;
pub mod generation;
pub mod languages;
pub mod markdown;
pub mod models;
pub mod phrases;
mod prompt;
pub mod publishing;
pub mod report;
pub mod runtime;
pub mod session;
pub mod tui;
pub mod vocabulary;

//! `lt`: a full Ligus client in the terminal.
//!
//! Ligus is short for ligustica, the Italian honeybee. `lt` is its own
//! device: it has its own keys, joins workspaces and signs like the desktop,
//! and works without a server. Today it runs against an in-memory fake
//! backend; `docs/ARCHITECTURE.md` has the plan for wiring the Ligus crates.

pub mod app;
pub mod backend;
pub mod cli;
pub mod clipboard;
pub mod completions;
pub mod config;
pub mod doctor;
pub mod help;
pub mod keymap;
pub mod paths;
pub mod plain;
pub mod setup;
pub mod termprobe;
pub mod theme;
pub mod theme_file;
#[rustfmt::skip]
#[allow(dead_code)]
pub mod theme_tokens;
pub mod ui;

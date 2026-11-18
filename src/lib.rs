//! `lt`: a full Ligus client in the terminal.
//!
//! Ligus is short for ligustica, the Italian honeybee. `lt` is its own
//! device: it has its own keys, joins workspaces and signs like the desktop,
//! and works without a server. Today it runs against an in-memory fake
//! backend; `docs/ARCHITECTURE.md` has the plan for wiring the Ligus crates.

pub mod app;
pub mod backend;
pub mod config;
pub mod keymap;
pub mod paths;
pub mod plain;
pub mod theme;
pub mod ui;

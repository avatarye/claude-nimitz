//! Admiralty: watch and control every claude-nimitz fleet on this machine.
//!
//! snapshot (nimitz-core) + panes (nimitz-mux) -> `view` node tree -> flat rows
//! -> `frames` lines; `app` holds state and keys, `render` draws, `run` is the
//! terminal loop. No Claude Code file parsing here: core does that.

pub mod actions;
pub mod backend;
pub mod app;
pub mod data;
pub mod detail;
pub mod fixtures;
pub mod forms;
pub mod frames;
pub mod herdr_plugin;
pub mod render;
pub mod run;
pub mod theme;
pub mod view;

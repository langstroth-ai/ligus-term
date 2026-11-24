//! App state, the reducer, and the loop that ties them to a terminal and a
//! backend.

pub mod input;
pub mod menu;
pub mod palette;
pub mod runtime;
pub mod state;
pub mod update;

pub use state::State;
pub use update::{Effect, Msg, update};

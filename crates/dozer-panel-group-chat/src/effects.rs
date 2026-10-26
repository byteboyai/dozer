//! Legacy module path for the effect executors; implementation is in `update`.
pub use crate::update::{
    Effect, on_activate, poll_if_due, run_effect, select_group, spawn_command,
    spawn_fetch_messages, spawn_load_groups,
};

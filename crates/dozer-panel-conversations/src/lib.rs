//! Shared conversations panel: workspace state, messages, service-backed updates,
//! summary polling, list/detail views, review data and protocol resources.
//! Products inject presentation and desktop facilities through neutral adapters.
pub const CONVERSATIONS_OWNER: &str = "dozer.conversations";
pub mod assets;
pub mod filter;
pub mod i18n;
pub mod review;
pub mod snapshot;

#[cfg(test)]
mod task3_tests;

pub mod adapter;
pub mod detail;
pub mod state;
pub mod summary;
pub mod update;
pub mod view;
pub use state::WorkspaceState;
pub use update::Message;
#[cfg(test)]
mod fakes;

/// Neutral panel logging, with the existing conversations target.
pub mod log {
    pub fn debug(message: &str, detail: &str) {
        tracing::debug!(target:"dozer::panel::conversations","{message}: {detail}");
    }
    pub fn warn(message: &str, detail: &str) {
        tracing::warn!(target:"dozer::panel::conversations","{message}: {detail}");
    }
}

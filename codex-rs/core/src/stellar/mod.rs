//! Stellar core kernel primitives shared between the TUI and CLI front-ends.
//!
//! The kernel is intentionally platform-agnostic: it expresses user intent as
//! [`StellarAction`] values scoped by [`StellarPersona`] profiles, validates
//! them via [`InputGuard`], and applies updates to the in-memory
//! [`StellarKernel`] state machine. UI layers (TUI, CLI bridge, automated
//! workflows) consume the resulting [`KernelSnapshot`] to render layouts or
//! emit structured events.

mod action;
mod event;
mod guard;
mod persona;
mod rbac;
mod snapshot;
mod state;

pub use action::StellarAction;
pub use action::StellarActionId;
pub use event::KernelEvent;
pub use event::StellarCliEvent;
pub use guard::GuardError;
pub use guard::InputGuard;
pub use persona::StellarPersona;
pub use rbac::is_action_allowed;
pub use snapshot::ConfidenceSnapshot;
pub use snapshot::GoldenPathHint;
pub use snapshot::KernelSnapshot;
pub use snapshot::LayoutMode;
pub use snapshot::PaneFocus;
pub use snapshot::RiskAlert;
pub use snapshot::RiskSeverity;
pub use snapshot::RunbookShortcut;
pub use state::ActionApplied;
pub use state::StellarKernel;

#[cfg(test)]
mod tests;

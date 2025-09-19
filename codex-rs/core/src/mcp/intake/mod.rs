//! Intake engine primitives for the MCP configuration workflow.
//! This layer is responsible for parsing user-provided sources,
//! tracking state transitions and exposing explainable insights.

pub mod cache;
pub mod detectors;
pub mod engine;
pub mod fingerprint_store;
pub mod parser;
pub mod policy;
pub mod preview;
pub mod reason_codes;
pub mod state;

pub use cache::SignalCache;
pub use detectors::DetectorRegistry;
pub use detectors::IntakeDetector;
pub use engine::IntakeEngine;
pub use fingerprint_store::FingerprintStatus;
pub use fingerprint_store::FingerprintStore;
pub use fingerprint_store::SharedFingerprintStore;
pub use parser::ParsedSource;
pub use parser::SourceKind;
pub use parser::SourceParser;
pub use reason_codes::ReasonCatalog;
pub use reason_codes::ReasonCode;
pub use reason_codes::ReasonCodeId;
pub use state::ConfidenceLevel;
pub use state::InsightSuggestion;
pub use state::IntakePhase;
pub use state::IntakeState;
pub use state::SourcePreview;

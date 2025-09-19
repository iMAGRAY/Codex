use serde::Deserialize;
use serde::Serialize;
use std::fmt;

/// Personas supported by the Stellar kernel. Personas drive default keymaps,
/// overlays and policy decisions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Hash)]
#[serde(rename_all = "kebab-case")]
pub enum StellarPersona {
    Operator,
    Sre,
    SecOps,
    PlatformEngineer,
    PartnerDeveloper,
    AssistiveBridge,
}

impl Default for StellarPersona {
    fn default() -> Self {
        StellarPersona::Operator
    }
}

impl fmt::Display for StellarPersona {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let label = match self {
            StellarPersona::Operator => "operator",
            StellarPersona::Sre => "sre",
            StellarPersona::SecOps => "secops",
            StellarPersona::PlatformEngineer => "platform-engineer",
            StellarPersona::PartnerDeveloper => "partner-developer",
            StellarPersona::AssistiveBridge => "assistive-bridge",
        };
        f.write_str(label)
    }
}

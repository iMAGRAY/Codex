use serde::Serialize;
use std::fmt;

#[derive(Debug, Clone, Copy, Serialize, PartialEq)]
pub enum ConfidenceFactor {
    Freshness,
    SourceTrust,
    SchemaValidity,
    TelemetryAlignment,
    UserOverrides,
}

impl fmt::Display for ConfidenceFactor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ConfidenceFactor::Freshness => write!(f, "Freshness"),
            ConfidenceFactor::SourceTrust => write!(f, "Source Trust"),
            ConfidenceFactor::SchemaValidity => write!(f, "Schema Validity"),
            ConfidenceFactor::TelemetryAlignment => write!(f, "Telemetry Alignment"),
            ConfidenceFactor::UserOverrides => write!(f, "User Overrides"),
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct FactorWeight {
    pub factor: ConfidenceFactor,
    pub weight: f32,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct ConfidenceInput {
    pub freshness: f32,
    pub source_trust: f32,
    pub schema_valid: f32,
    pub telemetry_alignment: f32,
    pub user_overrides: f32,
}

impl ConfidenceInput {
    pub fn clamp(self) -> Self {
        Self {
            freshness: self.freshness.clamp(0.0, 1.0),
            source_trust: self.source_trust.clamp(0.0, 1.0),
            schema_valid: self.schema_valid.clamp(0.0, 1.0),
            telemetry_alignment: self.telemetry_alignment.clamp(0.0, 1.0),
            user_overrides: self.user_overrides.clamp(0.0, 1.0),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ConfidenceBreakdown {
    pub factor: ConfidenceFactor,
    pub weight: f32,
    pub contribution: f32,
}

#[derive(Debug, Clone, Serialize)]
pub struct ConfidenceScore {
    pub value: f32,
    pub breakdown: Vec<ConfidenceBreakdown>,
}

#[derive(Debug, Clone)]
pub struct ConfidenceCalculator {
    weights: Vec<FactorWeight>,
}

impl ConfidenceCalculator {
    pub fn new(weights: Vec<FactorWeight>) -> Self {
        let sum: f32 = weights.iter().map(|w| w.weight).sum();
        let normalized = if sum.abs() > f32::EPSILON {
            weights
                .into_iter()
                .map(|w| FactorWeight {
                    factor: w.factor,
                    weight: (w.weight / sum).clamp(0.0, 1.0),
                })
                .collect()
        } else {
            vec![FactorWeight {
                factor: ConfidenceFactor::Freshness,
                weight: 1.0,
            }]
        };
        Self {
            weights: normalized,
        }
    }

    pub fn default() -> Self {
        Self::new(vec![
            FactorWeight {
                factor: ConfidenceFactor::Freshness,
                weight: 0.35,
            },
            FactorWeight {
                factor: ConfidenceFactor::SourceTrust,
                weight: 0.30,
            },
            FactorWeight {
                factor: ConfidenceFactor::SchemaValidity,
                weight: 0.20,
            },
            FactorWeight {
                factor: ConfidenceFactor::TelemetryAlignment,
                weight: 0.10,
            },
            FactorWeight {
                factor: ConfidenceFactor::UserOverrides,
                weight: 0.05,
            },
        ])
    }

    pub fn score(&self, input: ConfidenceInput) -> ConfidenceScore {
        let input = input.clamp();
        let mut value = 0.0f32;
        let mut breakdown = Vec::with_capacity(self.weights.len());
        for weight in &self.weights {
            let factor_value = match weight.factor {
                ConfidenceFactor::Freshness => input.freshness,
                ConfidenceFactor::SourceTrust => input.source_trust,
                ConfidenceFactor::SchemaValidity => input.schema_valid,
                ConfidenceFactor::TelemetryAlignment => input.telemetry_alignment,
                ConfidenceFactor::UserOverrides => input.user_overrides,
            };
            let contribution = factor_value * weight.weight;
            value += contribution;
            breakdown.push(ConfidenceBreakdown {
                factor: weight.factor,
                weight: weight.weight,
                contribution,
            });
        }
        ConfidenceScore {
            value: value.clamp(0.0, 1.0),
            breakdown,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn score_respects_weights() {
        let calc = ConfidenceCalculator::default();
        let result = calc.score(ConfidenceInput {
            freshness: 1.0,
            source_trust: 0.5,
            schema_valid: 0.2,
            telemetry_alignment: 0.8,
            user_overrides: 0.0,
        });
        assert!((result.value - 0.35 - 0.15 - 0.04 - 0.08).abs() < 1e-3);
        assert_eq!(result.breakdown.len(), 5);
    }
}

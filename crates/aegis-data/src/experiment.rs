//! Comparable experiment receipts with domain-specific metrics and uncertainty.
use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Experiment {
    pub id: String,
    pub base_model_sha256: String,
    pub model_sha256: String,
    pub training_data_sha256: String,
    pub evaluation_data_sha256: String,
    pub tokenizer_sha256: String,
    pub objective_sha256: String,
    pub schedule_sha256: String,
    pub evaluator_code_sha256: String,
    pub evaluator_config_sha256: String,
    pub decoding_sha256: String,
    pub seed: u64,
    pub updates: u64,
    pub wall_seconds: f64,
    pub metrics: BTreeMap<String, Metric>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Metric {
    pub value: f64,
    pub n: u64,
    pub direction: String,
    #[serde(default)]
    pub successes: Option<u64>,
}
#[derive(Serialize)]
pub struct Comparison {
    pub baseline: String,
    pub candidate: String,
    pub controlled: bool,
    pub confounders: Vec<String>,
    pub metrics: BTreeMap<String, Delta>,
    pub candidate_wall_seconds: f64,
}
#[derive(Serialize)]
pub struct Delta {
    pub baseline: Option<f64>,
    pub candidate: Option<f64>,
    pub delta: Option<f64>,
    pub status: String,
    pub baseline_ci95: Option<[f64; 2]>,
    pub candidate_ci95: Option<[f64; 2]>,
}
fn sha(v: &str) -> bool {
    v.len() == 64 && v.bytes().all(|b| b.is_ascii_hexdigit())
}
impl Experiment {
    pub fn validate(&self) -> Result<()> {
        if self.id.is_empty()
            || !self.wall_seconds.is_finite()
            || self.wall_seconds < 0.
            || self.metrics.is_empty()
        {
            bail!("invalid experiment")
        }
        for hash in [
            &self.base_model_sha256,
            &self.model_sha256,
            &self.training_data_sha256,
            &self.evaluation_data_sha256,
            &self.tokenizer_sha256,
            &self.objective_sha256,
            &self.schedule_sha256,
            &self.evaluator_code_sha256,
            &self.evaluator_config_sha256,
            &self.decoding_sha256,
        ] {
            if !sha(hash) {
                bail!("missing SHA256 identity")
            }
        }
        for m in self.metrics.values() {
            if !m.value.is_finite() || m.n == 0 || !["min", "max"].contains(&m.direction.as_str()) {
                bail!("invalid metric")
            }
            if let Some(s) = m.successes {
                if s > m.n || (m.value - s as f64 / m.n as f64).abs() > 1e-9 {
                    bail!("binomial value disagrees with counts")
                }
            }
        }
        Ok(())
    }
}
fn ci(m: &Metric) -> Option<[f64; 2]> {
    m.successes.map(|s| {
        let n = m.n as f64;
        let p = s as f64 / n;
        let z = 1.959963984540054;
        let den = 1. + z * z / n;
        let c = (p + z * z / (2. * n)) / den;
        let h = z * (p * (1. - p) / n + z * z / (4. * n * n)).sqrt() / den;
        [(c - h).max(0.), (c + h).min(1.)]
    })
}
pub fn compare(a: &Experiment, b: &Experiment) -> Result<Comparison> {
    a.validate()?;
    b.validate()?;
    let mut confounders = Vec::new();
    macro_rules! same {
        ($f:ident) => {
            if a.$f != b.$f {
                confounders.push(stringify!($f).into())
            }
        };
    }
    same!(base_model_sha256);
    same!(training_data_sha256);
    same!(evaluation_data_sha256);
    same!(tokenizer_sha256);
    same!(objective_sha256);
    same!(schedule_sha256);
    same!(evaluator_code_sha256);
    same!(evaluator_config_sha256);
    same!(decoding_sha256);
    same!(seed);
    same!(updates);
    let mut metrics = BTreeMap::new();
    let names: std::collections::BTreeSet<_> = a.metrics.keys().chain(b.metrics.keys()).collect();
    for name in names {
        let (old, new) = (a.metrics.get(name), b.metrics.get(name));
        let pair = old.zip(new);
        let compatible = pair.is_some_and(|(x, y)| {
            x.n == y.n
                && x.direction == y.direction
                && a.evaluation_data_sha256 == b.evaluation_data_sha256
                && a.evaluator_code_sha256 == b.evaluator_code_sha256
                && a.evaluator_config_sha256 == b.evaluator_config_sha256
                && a.decoding_sha256 == b.decoding_sha256
        });
        let delta = pair.filter(|_| compatible).map(|(x, y)| y.value - x.value);
        let status = if !compatible {
            "not_comparable"
        } else if delta == Some(0.) {
            "unchanged"
        } else if pair.is_some_and(|(x, y)| {
            if x.direction == "min" {
                y.value > x.value
            } else {
                y.value < x.value
            }
        }) {
            "improved"
        } else {
            "regressed"
        };
        // Direction min means larger is better (a minimum release threshold).
        metrics.insert(
            name.clone(),
            Delta {
                baseline: old.map(|m| m.value),
                candidate: new.map(|m| m.value),
                delta,
                status: status.into(),
                baseline_ci95: old.and_then(ci),
                candidate_ci95: new.and_then(ci),
            },
        );
    }
    Ok(Comparison {
        baseline: a.id.clone(),
        candidate: b.id.clone(),
        controlled: confounders.is_empty(),
        confounders,
        metrics,
        candidate_wall_seconds: b.wall_seconds,
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    fn experiment(value: f64) -> Experiment {
        Experiment {
            id: "fixture".into(),
            base_model_sha256: "a".repeat(64),
            model_sha256: "b".repeat(64),
            training_data_sha256: "c".repeat(64),
            evaluation_data_sha256: "d".repeat(64),
            tokenizer_sha256: "e".repeat(64),
            objective_sha256: "f".repeat(64),
            schedule_sha256: "0".repeat(64),
            evaluator_code_sha256: "1".repeat(64),
            evaluator_config_sha256: "2".repeat(64),
            decoding_sha256: "3".repeat(64),
            seed: 7,
            updates: 10,
            wall_seconds: 1.,
            metrics: BTreeMap::from([(
                "code/accuracy".into(),
                Metric {
                    value,
                    n: 100,
                    direction: "min".into(),
                    successes: Some((value * 100.) as u64),
                },
            )]),
        }
    }
    #[test]
    fn compare_and_confound() {
        let a = experiment(0.5);
        let mut b = experiment(0.6);
        b.updates = 11;
        let r = compare(&a, &b).unwrap();
        assert!(!r.controlled);
        assert_eq!(r.metrics["code/accuracy"].status, "improved");
        assert!(r.metrics["code/accuracy"].candidate_ci95.unwrap()[0] < 0.6);
    }
    #[test]
    fn changed_evaluator_is_not_comparable() {
        let a = experiment(0.5);
        let mut b = experiment(0.6);
        b.decoding_sha256 = "a".repeat(64);
        assert_eq!(
            compare(&a, &b).unwrap().metrics["code/accuracy"].status,
            "not_comparable"
        );
    }
    #[test]
    fn counts_must_agree() {
        let mut a = experiment(0.5);
        a.metrics.get_mut("code/accuracy").unwrap().successes = Some(90);
        assert!(a.validate().is_err())
    }
}

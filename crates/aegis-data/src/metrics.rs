use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
#[derive(Deserialize)]
pub struct Classification {
    pub probabilities: Vec<Vec<f64>>,
    pub labels: Vec<usize>,
}
#[derive(Serialize, Debug)]
pub struct ClassScores {
    pub n: usize,
    pub accuracy: f64,
    pub cross_entropy: f64,
    pub brier: f64,
    pub macro_f1: f64,
    pub confusion: Vec<Vec<u64>>,
}
pub fn classification(x: &Classification) -> Result<ClassScores> {
    let n = x.labels.len();
    if n == 0 || x.probabilities.len() != n {
        bail!("empty or mismatched labels")
    }
    let k = x.probabilities[0].len();
    if k < 2 {
        bail!("need at least two classes")
    }
    let mut c = vec![vec![0u64; k]; k];
    let (mut ce, mut brier, mut correct) = (0., 0., 0u64);
    for (p, &y) in x.probabilities.iter().zip(&x.labels) {
        if p.len() != k
            || y >= k
            || p.iter()
                .any(|&v| !v.is_finite() || !(0.0..=1.0).contains(&v))
            || (p.iter().sum::<f64>() - 1.).abs() > 1e-6
        {
            bail!("invalid probabilities or label")
        }
        let predicted = (0..k)
            .max_by(|&a, &b| p[a].total_cmp(&p[b]).then_with(|| b.cmp(&a)))
            .unwrap();
        c[y][predicted] += 1;
        correct += u64::from(y == predicted);
        ce -= p[y].max(1e-15).ln();
        brier += p
            .iter()
            .enumerate()
            .map(|(i, v)| (v - if i == y { 1. } else { 0. }).powi(2))
            .sum::<f64>();
    }
    let f1 = (0..k)
        .map(|i| {
            let tp = c[i][i] as f64;
            let fp = (0..k).map(|j| c[j][i]).sum::<u64>() as f64 - tp;
            let fn_ = c[i].iter().sum::<u64>() as f64 - tp;
            let den = 2. * tp + fp + fn_;
            if den == 0. {
                0.
            } else {
                2. * tp / den
            }
        })
        .sum::<f64>()
        / k as f64;
    Ok(ClassScores {
        n,
        accuracy: correct as f64 / n as f64,
        cross_entropy: ce / n as f64,
        brier: brier / n as f64,
        macro_f1: f1,
        confusion: c,
    })
}
#[derive(Deserialize)]
pub struct Regression {
    pub predictions: Vec<f64>,
    pub targets: Vec<f64>,
}
#[derive(Serialize, Debug)]
pub struct RegressionScores {
    pub n: usize,
    pub mse: f64,
    pub rmse: f64,
    pub mae: f64,
    pub r_squared: Option<f64>,
}
pub fn regression(x: &Regression) -> Result<RegressionScores> {
    let n = x.targets.len();
    if n == 0
        || x.predictions.len() != n
        || x.targets
            .iter()
            .chain(&x.predictions)
            .any(|v| !v.is_finite())
    {
        bail!("invalid regression inputs")
    }
    let mean = x.targets.iter().sum::<f64>() / n as f64;
    let sse = x
        .targets
        .iter()
        .zip(&x.predictions)
        .map(|(a, b)| (a - b).powi(2))
        .sum::<f64>();
    let total = x.targets.iter().map(|v| (v - mean).powi(2)).sum::<f64>();
    let mae = x
        .targets
        .iter()
        .zip(&x.predictions)
        .map(|(a, b)| (a - b).abs())
        .sum::<f64>()
        / n as f64;
    if !sse.is_finite() || !total.is_finite() || !mae.is_finite() {
        bail!("numeric overflow")
    }
    Ok(RegressionScores {
        n,
        mse: sse / n as f64,
        rmse: (sse / n as f64).sqrt(),
        mae,
        r_squared: if total > 0. {
            Some(1. - sse / total)
        } else {
            None
        },
    })
}
#[derive(Deserialize)]
pub struct Gate {
    pub name: String,
    pub direction: String,
    pub threshold: f64,
    pub measured: Option<f64>,
    pub metric_pointer: Option<String>,
    pub evidence_sha256: Option<String>,
    pub evidence_file: Option<std::path::PathBuf>,
}
#[derive(Serialize)]
pub struct Verdict {
    pub name: String,
    pub pass: bool,
    pub reason: String,
    pub measured: Option<f64>,
}
pub fn gates(gates: &[Gate]) -> Result<Vec<Verdict>> {
    if gates.is_empty() {
        bail!("empty gate suite")
    }
    let mut names = std::collections::HashSet::new();
    let mut out = Vec::new();
    for g in gates {
        if !names.insert(&g.name)
            || !g.threshold.is_finite()
            || !["min", "max"].contains(&g.direction.as_str())
        {
            bail!("duplicate or invalid gate")
        }
        let measurement = (|| -> Option<f64> {
            let path = g.evidence_file.as_ref()?;
            if std::fs::metadata(path).ok()?.len() > 16 * 1024 * 1024 {
                return None;
            }
            let bytes = std::fs::read(path).ok()?;
            if crate::ingest::hash(&bytes) != *g.evidence_sha256.as_ref()? {
                return None;
            }
            let result: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
            let value = result.pointer(g.metric_pointer.as_ref()?)?.as_f64()?;
            if !value.is_finite() || g.measured.is_some_and(|claimed| claimed != value) {
                return None;
            }
            Some(value)
        })();
        let pass = measurement.is_some_and(|v| {
            if g.direction == "min" {
                v >= g.threshold
            } else {
                v <= g.threshold
            }
        });
        out.push(Verdict {
            name: g.name.clone(),
            pass,
            measured: measurement,
            reason: if measurement.is_none() {
                "missing, mismatched or invalid metric evidence"
            } else if pass {
                "artifact-derived measurement meets threshold"
            } else {
                "artifact-derived measurement fails threshold"
            }
            .into(),
        });
    }
    Ok(out)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn known_classification() {
        let s = classification(&Classification {
            probabilities: vec![vec![0.9, 0.1], vec![0.8, 0.2]],
            labels: vec![0, 1],
        })
        .unwrap();
        assert_eq!(s.accuracy, 0.5);
        assert!((s.cross_entropy - (-(0.9f64.ln() + 0.2f64.ln()) / 2.)).abs() < 1e-12);
        assert!((s.macro_f1 - 1. / 3.).abs() < 1e-12)
    }
    #[test]
    fn invalid_probabilities() {
        assert!(classification(&Classification {
            probabilities: vec![vec![0.5, 0.6]],
            labels: vec![0]
        })
        .is_err())
    }
    #[test]
    fn known_regression() {
        let s = regression(&Regression {
            predictions: vec![2., 4.],
            targets: vec![1., 2.],
        })
        .unwrap();
        assert_eq!(s.mse, 2.5);
        assert_eq!(s.r_squared, Some(-9.));
    }
    #[test]
    fn constant_r2_undefined() {
        assert!(regression(&Regression {
            predictions: vec![1.],
            targets: vec![1.]
        })
        .unwrap()
        .r_squared
        .is_none())
    }
    #[test]
    fn valid_evidence_and_tamper() {
        let t = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(t.path(), br#"{"accuracy":1.0}"#).unwrap();
        let g = Gate {
            name: "quality".into(),
            direction: "min".into(),
            threshold: 0.8,
            measured: Some(1.),
            metric_pointer: Some("/accuracy".into()),
            evidence_sha256: Some(crate::ingest::hash(br#"{"accuracy":1.0}"#)),
            evidence_file: Some(t.path().into()),
        };
        assert!(gates(&[g]).unwrap()[0].pass);
        let bad = Gate {
            name: "quality".into(),
            direction: "min".into(),
            threshold: 0.8,
            measured: Some(1.),
            metric_pointer: Some("/accuracy".into()),
            evidence_sha256: Some("0".repeat(64)),
            evidence_file: Some(t.path().into()),
        };
        assert!(!gates(&[bad]).unwrap()[0].pass);
    }
    #[test]
    fn separately_supplied_score_cannot_forge_gate() {
        let t = tempfile::NamedTempFile::new().unwrap();
        let bytes = br#"{"accuracy":0.2}"#;
        std::fs::write(t.path(), bytes).unwrap();
        let g = Gate {
            name: "quality".into(),
            direction: "min".into(),
            threshold: 0.8,
            measured: Some(1.),
            metric_pointer: Some("/accuracy".into()),
            evidence_sha256: Some(crate::ingest::hash(bytes)),
            evidence_file: Some(t.path().into()),
        };
        assert!(!gates(&[g]).unwrap()[0].pass);
    }
    #[test]
    fn missing_evidence_fails() {
        let g = Gate {
            name: "quality".into(),
            direction: "min".into(),
            threshold: 0.8,
            measured: Some(1.),
            metric_pointer: Some("/accuracy".into()),
            evidence_sha256: None,
            evidence_file: None,
        };
        assert!(!gates(&[g]).unwrap()[0].pass)
    }
}

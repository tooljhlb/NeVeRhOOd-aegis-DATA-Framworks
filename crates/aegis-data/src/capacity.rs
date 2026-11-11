//! Atomic capacity reservations for one API server process.
use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Limits {
    pub max_runs: u64,
    pub cpu: u64,
    pub memory_mib: u64,
}
impl Limits {
    pub fn validate(self) -> Result<Self> {
        if !(1..=64).contains(&self.max_runs)
            || !(1..=65536).contains(&self.cpu)
            || !(64..=67108864).contains(&self.memory_mib)
        {
            bail!("invalid API capacity limits")
        }
        Ok(self)
    }
}
#[derive(Default, Clone, Copy, Serialize)]
struct Usage {
    runs: u64,
    cpu: u64,
    memory_mib: u64,
}
#[derive(Clone)]
pub struct Pool {
    limits: Limits,
    used: Arc<Mutex<Usage>>,
}
pub struct Reservation {
    pool: Pool,
    cpu: u64,
    memory_mib: u64,
}
impl Pool {
    pub fn new(limits: Limits) -> Result<Self> {
        Ok(Self {
            limits: limits.validate()?,
            used: Arc::new(Mutex::new(Usage::default())),
        })
    }
    pub fn reserve(&self, cpu: u64, memory_mib: u64) -> Result<Reservation> {
        let mut used = self
            .used
            .lock()
            .map_err(|_| anyhow::anyhow!("capacity lock poisoned"))?;
        if cpu == 0
            || memory_mib == 0
            || cpu > self.limits.cpu
            || memory_mib > self.limits.memory_mib
            || used.runs >= self.limits.max_runs
            || used.cpu > self.limits.cpu - cpu
            || used.memory_mib > self.limits.memory_mib - memory_mib
        {
            bail!("API capacity unavailable; retry after an active run completes")
        }
        used.runs += 1;
        used.cpu += cpu;
        used.memory_mib += memory_mib;
        Ok(Reservation {
            pool: self.clone(),
            cpu,
            memory_mib,
        })
    }
    pub fn snapshot(&self) -> Result<serde_json::Value> {
        let used = self
            .used
            .lock()
            .map_err(|_| anyhow::anyhow!("capacity lock poisoned"))?;
        Ok(serde_json::json!({"limits":self.limits,"reserved":*used,
            "scope":"this API process only; not machine-wide or distributed scheduling"}))
    }
}
impl Drop for Reservation {
    fn drop(&mut self) {
        if let Ok(mut used) = self.pool.used.lock() {
            used.runs -= 1;
            used.cpu -= self.cpu;
            used.memory_mib -= self.memory_mib;
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejection_is_atomic_and_release_restores_capacity() {
        let pool = Pool::new(Limits {
            max_runs: 2,
            cpu: 2,
            memory_mib: 256,
        })
        .unwrap();
        let a = pool.reserve(1, 128).unwrap();
        assert!(pool.reserve(2, 128).is_err());
        assert_eq!(pool.snapshot().unwrap()["reserved"]["cpu"], 1);
        let b = pool.reserve(1, 128).unwrap();
        assert!(pool.reserve(1, 1).is_err());
        drop(a);
        drop(b);
        assert_eq!(pool.snapshot().unwrap()["reserved"]["runs"], 0);
        assert!(pool.reserve(u64::MAX, 128).is_err());
        assert!(pool.reserve(2, 256).is_ok());
    }
    #[test]
    fn concurrent_claims_cannot_overbook() {
        let pool = Pool::new(Limits {
            max_runs: 1,
            cpu: 1,
            memory_mib: 64,
        })
        .unwrap();
        let barrier = Arc::new(std::sync::Barrier::new(8));
        let threads: Vec<_> = (0..8)
            .map(|_| {
                let pool = pool.clone();
                let barrier = barrier.clone();
                std::thread::spawn(move || {
                    let claim = pool.reserve(1, 64).ok();
                    barrier.wait();
                    claim.is_some()
                })
            })
            .collect();
        assert_eq!(
            threads
                .into_iter()
                .map(|t| usize::from(t.join().unwrap()))
                .sum::<usize>(),
            1
        );
        assert_eq!(pool.snapshot().unwrap()["reserved"]["runs"], 0);
    }
}
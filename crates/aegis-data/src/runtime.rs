//! Shared admission policy for CLI and API execution. Limits are per run.
use crate::operator::Plan;
use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Profile {
    pub schema_version: u32,
    pub id: String,
    pub require_container: bool,
    pub allowed_programs: BTreeSet<String>,
    pub allowed_images: BTreeSet<String>,
    pub max_slots: usize,
    pub max_wall_seconds: u64,
    pub max_attempts: usize,
    pub max_stage_seconds: u64,
    pub max_cpu: u64,
    pub max_memory_mib: u64,
}
impl Profile {
    pub fn validate(&self) -> Result<()> {
        if self.schema_version != 1
            || self.id.is_empty()
            || self.id.len() > 96
            || self.allowed_programs.is_empty()
            || self.allowed_programs.iter().any(|p| p.is_empty())
            || !(1..=16).contains(&self.max_slots)
            || !(1..=86400).contains(&self.max_wall_seconds)
            || !(1..=5).contains(&self.max_attempts)
            || !(1..=3600).contains(&self.max_stage_seconds)
            || !(1..=1024).contains(&self.max_cpu)
            || !(64..=1048576).contains(&self.max_memory_mib)
            || (self.require_container && self.allowed_images.is_empty())
        {
            bail!("invalid runtime profile")
        }
        Ok(())
    }
    pub fn admit(&self, plan: &Plan) -> Result<Value> {
        self.validate()?;
        plan.validate()?;
        if plan.max_slots > self.max_slots || plan.max_wall_seconds > self.max_wall_seconds {
            bail!("plan exceeds runtime concurrency or wall-time budget")
        }
        let mut resources = Vec::new();
        for stage in &plan.stages {
            if !self.allowed_programs.contains(&stage.program)
                || stage.max_attempts > self.max_attempts
                || stage.timeout_seconds > self.max_stage_seconds
            {
                bail!(
                    "stage {} violates executable, retry or timeout policy",
                    stage.id
                )
            }
            match &stage.container {
                Some(c) => {
                    if !self.allowed_images.contains(&c.image) {
                        bail!("stage {} uses an unapproved image", stage.id)
                    }
                    resources.push((u64::from(c.cpus), c.memory_mib));
                }
                None if self.require_container => {
                    bail!("stage {} requires container isolation", stage.id)
                }
                None => {}
            }
        }
        // Conservative upper bound: at most max_slots stages can run concurrently.
        // Ignoring DAG exclusions and multi-slot stages can overestimate, never undercount.
        let mut cpus: Vec<_> = resources.iter().map(|r| r.0).collect();
        let mut memory: Vec<_> = resources.iter().map(|r| r.1).collect();
        cpus.sort_unstable_by(|a, b| b.cmp(a));
        memory.sort_unstable_by(|a, b| b.cmp(a));
        let cpu: u64 = cpus.iter().take(plan.max_slots).sum();
        let memory: u64 = memory.iter().take(plan.max_slots).sum();
        if cpu > self.max_cpu || memory > self.max_memory_mib {
            bail!("concurrent container resource envelope exceeds profile")
        }
        Ok(json!({"admitted":true,"profile":self.id,
            "profile_sha256":format!("{:x}", Sha256::digest(serde_json::to_vec(self)?)),
            "plan_sha256":format!("{:x}", Sha256::digest(serde_json::to_vec(plan)?)),"container_cpu_upper_bound":cpu,
            "container_memory_mib_upper_bound":memory,"scope":"per-run admission; host processes and other runs are not resource reservations"}))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> (Profile, Plan) {
        let image = format!("busybox@sha256:{}", "a".repeat(64));
        let profile = Profile {
            schema_version: 1,
            id: "isolated".into(),
            require_container: true,
            allowed_programs: ["sh".into()].into(),
            allowed_images: [image.clone()].into(),
            max_slots: 2,
            max_wall_seconds: 120,
            max_attempts: 1,
            max_stage_seconds: 60,
            max_cpu: 2,
            max_memory_mib: 256,
        };
        let plan = serde_json::from_value(json!({"run_id":"test","workspace":".","mode":"bounded",
            "max_slots":2,"max_wall_seconds":120,"allowed_operations":["test"],
            "stages":[{"id":"a","operation":"test","program":"sh","args":["test.sh"],
            "timeout_seconds":60,"container":{"image":image,"cpus":1,"memory_mib":128,"pids":32}}]})).unwrap();
        (profile, plan)
    }
    #[test]
    fn admits_and_rejects_execution_policy_changes() {
        let (profile, mut plan) = fixture();
        assert!(profile.admit(&plan).is_ok());
        plan.stages[0].container = None;
        assert!(profile.admit(&plan).is_err());
        let (_, mut plan) = fixture();
        plan.stages[0].program = "bash".into();
        assert!(profile.admit(&plan).is_err());
        let (_, mut plan) = fixture();
        plan.stages[0].max_attempts = 2;
        assert!(profile.admit(&plan).is_err());
    }
    #[test]
    fn bounds_concurrent_resources_and_images() {
        let (profile, mut plan) = fixture();
        let mut second = plan.stages[0].clone();
        second.id = "b".into();
        plan.stages.push(second);
        assert!(profile.admit(&plan).is_ok());
        plan.stages[1].container.as_mut().unwrap().memory_mib = 256;
        assert!(profile.admit(&plan).is_err());
        let (_, mut plan) = fixture();
        plan.stages[0].container.as_mut().unwrap().image =
            format!("other@sha256:{}", "b".repeat(64));
        assert!(profile.admit(&plan).is_err());
    }
}

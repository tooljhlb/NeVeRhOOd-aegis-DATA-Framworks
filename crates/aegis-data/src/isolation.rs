//! Opt-in OCI execution. Pinned images only, no network, bounded resources.
use crate::code::Check;
use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{path::Path, process::Command};
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Container {
    pub image: String,
    pub memory_mib: u64,
    pub cpus: u32,
    pub pids: u32,
}
impl Container {
    pub fn validate(&self) -> Result<()> {
        let (_, digest) = self
            .image
            .rsplit_once("@sha256:")
            .ok_or_else(|| anyhow::anyhow!("container image must be digest pinned"))?;
        if self.image.starts_with('-')
            || self.image.chars().any(char::is_whitespace)
            || digest.len() != 64
            || !digest.bytes().all(|b| b.is_ascii_hexdigit())
            || !(64..=65536).contains(&self.memory_mib)
            || !(1..=64).contains(&self.cpus)
            || !(16..=4096).contains(&self.pids)
        {
            bail!("invalid isolation limits")
        }
        Ok(())
    }
    pub fn name(run: &str, stage: &str) -> String {
        format!(
            "aegis-{:x}",
            Sha256::digest(format!("{run}\0{stage}").as_bytes())
        )[..38]
            .into()
    }
    pub fn wrap(&self, root: &Path, name: &str, check: &Check) -> Result<Check> {
        self.validate()?;
        let root = root.canonicalize()?;
        if root.to_string_lossy().contains(',') {
            bail!("container workspace path cannot contain comma")
        }
        let mut args: Vec<String> = [
            "--host=unix:///var/run/docker.sock",
            "run",
            "--rm",
            "--pull=never",
            "--name",
            name,
            "--network=none",
            "--read-only",
            "--cap-drop=ALL",
            "--security-opt=no-new-privileges",
            "--workdir=/workspace",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        args.extend([
            "--memory".into(),
            format!("{}m", self.memory_mib),
            "--memory-swap".into(),
            format!("{}m", self.memory_mib),
            "--cpus".into(),
            self.cpus.to_string(),
            "--pids-limit".into(),
            self.pids.to_string(),
            "--tmpfs".into(),
            "/tmp:rw,nosuid,nodev,noexec,size=67108864".into(),
            "--mount".into(),
            format!("type=bind,src={},dst=/workspace", root.display()),
        ]);
        #[cfg(unix)]
        args.extend([
            "--user".into(),
            format!("{}:{}", unsafe { libc::geteuid() }, unsafe {
                libc::getegid()
            }),
        ]);
        args.extend([
            "--entrypoint".into(),
            check.program.clone(),
            self.image.clone(),
        ]);
        args.extend(check.args.clone());
        Ok(Check {
            id: check.id.clone(),
            kind: check.kind.clone(),
            program: "docker".into(),
            args,
            project_marker: None,
        })
    }
    pub fn cleanup(name: &str) -> Result<()> {
        // Called only for the deterministic AEGIS-owned container name. No broad prune.
        if !docker(&["info", "--format", "{{.ServerVersion}}"])? {
            bail!("Docker daemon unavailable; isolation cannot fall back to host")
        }
        if !docker(&["rm", "--force", name])? && docker(&["inspect", name])? {
            bail!("container cleanup failed")
        }
        Ok(())
    }
}
fn docker(args: &[&str]) -> Result<bool> {
    let mut command = Command::new("docker");
    command.env_clear();
    if let Some(path) = std::env::var_os("PATH") {
        command.env("PATH", path);
    }
    let mut child = command
        .arg("--host=unix:///var/run/docker.sock")
        .args(args)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()?;
    let start = std::time::Instant::now();
    loop {
        if let Some(status) = child.try_wait()? {
            return Ok(status.success());
        }
        if start.elapsed() > std::time::Duration::from_secs(10) {
            let _ = child.kill();
            let _ = child.wait();
            bail!("Docker control deadline exceeded")
        }
        std::thread::sleep(std::time::Duration::from_millis(25));
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pinned_limits_only() {
        let mut c = Container {
            image: format!("busybox@sha256:{}", "a".repeat(64)),
            memory_mib: 128,
            cpus: 1,
            pids: 32,
        };
        assert!(c.validate().is_ok());
        c.image = "busybox:latest".into();
        assert!(c.validate().is_err());
    }
}

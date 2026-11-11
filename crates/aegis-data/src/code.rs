//! Extensible polyglot check registry. Commands run without a shell, on trusted
//! projects only. Resource limits here are not an untrusted-code sandbox.
use anyhow::{bail, Context, Result};
use ignore::WalkBuilder;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};
#[derive(Deserialize, Serialize, Clone)]
#[serde(deny_unknown_fields)]
pub struct Registry {
    pub version: u32,
    pub languages: Vec<Language>,
}
#[derive(Deserialize, Serialize, Clone)]
#[serde(deny_unknown_fields)]
pub struct Language {
    pub id: String,
    pub extensions: Vec<String>,
    pub filenames: Vec<String>,
    pub checks: Vec<Check>,
}
#[derive(Deserialize, Serialize, Clone)]
#[serde(deny_unknown_fields)]
pub struct Check {
    pub id: String,
    pub kind: String,
    pub program: String,
    pub args: Vec<String>,
    #[serde(default)]
    pub project_marker: Option<String>,
}
#[derive(Serialize)]
pub struct Inventory {
    pub root: String,
    pub registry_sha256: String,
    pub files: Vec<Source>,
    pub ignored_large: Vec<String>,
}
#[derive(Serialize)]
pub struct Source {
    pub path: String,
    pub language: String,
    pub sha256: String,
}
#[derive(Serialize, Debug)]
pub struct CheckResult {
    pub check: String,
    pub language: String,
    pub path: String,
    pub status: String,
    pub command: Vec<String>,
    pub exit_code: Option<i32>,
    pub seconds: f64,
    pub output: String,
    pub output_truncated: bool,
}
#[derive(Serialize)]
pub struct Report {
    pub schema_version: u32,
    pub inventory: Inventory,
    pub checks: Vec<CheckResult>,
    pub summary: BTreeMap<String, usize>,
    pub complete: bool,
    pub execution_mode: String,
    pub selected_languages: Vec<String>,
}
pub fn registry(path: &Path) -> Result<Registry> {
    let r: Registry = serde_json::from_slice(&fs::read(path)?)?;
    if r.version != 1 || r.languages.is_empty() {
        bail!("invalid registry version or empty registry")
    }
    let mut ids = BTreeSet::new();
    let mut mappings = BTreeSet::new();
    for l in &r.languages {
        if l.id.is_empty() || !ids.insert(&l.id) {
            bail!("duplicate/empty language")
        }
        for e in &l.extensions {
            if e.is_empty() || !mappings.insert(format!("ext:{e}")) {
                bail!("duplicate/empty extension")
            }
        }
        for f in &l.filenames {
            if f.is_empty() || !mappings.insert(format!("name:{f}")) {
                bail!("duplicate/empty filename")
            }
        }
        let mut checks = BTreeSet::new();
        for c in &l.checks {
            if c.program.is_empty()
                || !checks.insert(&c.id)
                || !["lint", "format", "syntax", "type", "test"].contains(&c.kind.as_str())
            {
                bail!("invalid check")
            }
        }
    }
    Ok(r)
}
pub fn scan(root: &Path, registry_path: &Path) -> Result<(Inventory, Registry)> {
    let root = root.canonicalize()?;
    if !root.is_dir() {
        bail!("root must be a directory")
    }
    let r = registry(registry_path)?;
    let mut files = Vec::new();
    let mut large = Vec::new();
    let mut walker = WalkBuilder::new(&root);
    walker
        .hidden(false)
        .follow_links(false)
        .require_git(false)
        .filter_entry(|e| {
            !e.file_type().is_some_and(|t| t.is_dir())
                || ![
                    ".git",
                    "target",
                    "build",
                    "build-cuda",
                    "node_modules",
                    "vendor",
                    ".venv",
                    "__pycache__",
                ]
                .contains(&e.file_name().to_str().unwrap_or(""))
        });
    for entry in walker.build() {
        let e = entry?;
        if !e.file_type().is_some_and(|t| t.is_file()) {
            continue;
        }
        let p = e.path();
        let name = p.file_name().and_then(|s| s.to_str()).unwrap_or("");
        let ext = p.extension().and_then(|s| s.to_str()).unwrap_or("");
        let Some(l) = r
            .languages
            .iter()
            .find(|l| l.filenames.iter().any(|v| v == name))
            .or_else(|| {
                r.languages
                    .iter()
                    .find(|l| l.extensions.iter().any(|v| v == ext))
            })
        else {
            continue;
        };
        let relative = p
            .strip_prefix(&root)?
            .to_str()
            .context("non-UTF8 source path")?
            .to_owned();
        if e.metadata()?.len() > 2 * 1024 * 1024 {
            large.push(relative);
            continue;
        }
        files.push(Source {
            path: relative,
            language: l.id.clone(),
            sha256: crate::ingest::hash(&fs::read(p)?),
        });
    }
    files.sort_by(|a, b| a.path.cmp(&b.path));
    large.sort();
    Ok((
        Inventory {
            root: root.to_string_lossy().into(),
            registry_sha256: crate::ingest::hash(&fs::read(registry_path)?),
            files,
            ignored_large: large,
        },
        r,
    ))
}
const OUTPUT_LIMIT: usize = 64 * 1024;
fn drain(mut input: impl Read, mut log: Option<std::fs::File>) -> (Vec<u8>, bool) {
    let mut logged = 0usize;
    let mut bytes = Vec::new();
    let mut truncated = false;
    let mut buffer = [0u8; 8192];
    while let Ok(n) = input.read(&mut buffer) {
        if n == 0 {
            break;
        }
        if let Some(file) = log.as_mut() {
            let amount = n.min((16 * 1024 * 1024usize).saturating_sub(logged));
            if file.write_all(&buffer[..amount]).is_err() {
                truncated = true;
            }
            logged += amount;
            if amount < n {
                truncated = true;
            }
        }
        let keep = n.min(OUTPUT_LIMIT - bytes.len());
        bytes.extend_from_slice(&buffer[..keep]);
        truncated |= keep < n;
    }
    (bytes, truncated)
}
#[cfg(unix)]
pub fn run(
    root: &Path,
    check: &Check,
    language: &str,
    path: &str,
    timeout: Duration,
) -> Result<CheckResult> {
    run_observed(root, check, language, path, timeout, None, None)
}
#[cfg(unix)]
pub fn run_observed(
    root: &Path,
    check: &Check,
    language: &str,
    path: &str,
    timeout: Duration,
    logs: Option<&Path>,
    cancel: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,
) -> Result<CheckResult> {
    use std::os::unix::process::CommandExt;
    let begin = Instant::now();
    let args: Vec<_> = check
        .args
        .iter()
        .map(|a| a.replace("{file}", &format!("./{path}")))
        .collect();
    let mut command = vec![check.program.clone()];
    command.extend(args.clone());
    let mut cmd = Command::new(&check.program);
    cmd.args(&args)
        .current_dir(root)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env_clear()
        .process_group(0);
    for name in [
        "PATH",
        "HOME",
        "CARGO_HOME",
        "RUSTUP_HOME",
        "TMPDIR",
        "SYSTEMROOT",
    ] {
        if let Some(v) = std::env::var_os(name) {
            cmd.env(name, v);
        }
    }
    cmd.env("NO_COLOR", "1");
    #[cfg(target_os = "linux")]
    {
        let parent = std::process::id();
        unsafe {
            cmd.pre_exec(move || {
                if libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL) != 0 {
                    return Err(std::io::Error::last_os_error());
                }
                if libc::getppid() != parent as i32 {
                    return Err(std::io::Error::other("supervisor exited"));
                }
                Ok(())
            });
        }
    }

    let stdout_log = logs
        .map(|p| {
            std::fs::create_dir_all(p)?;
            std::fs::File::create(p.join("stdout.log"))
        })
        .transpose()?;
    let stderr_log = logs
        .map(|p| std::fs::File::create(p.join("stderr.log")))
        .transpose()?;
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            return Ok(CheckResult {
                check: check.id.clone(),
                language: language.into(),
                path: path.into(),
                status: if e.kind() == std::io::ErrorKind::NotFound {
                    "unavailable"
                } else {
                    "error"
                }
                .into(),
                command,
                exit_code: None,
                seconds: begin.elapsed().as_secs_f64(),
                output: e.to_string(),
                output_truncated: false,
            })
        }
    };
    let stdout = child.stdout.take().unwrap();
    let stderr = child.stderr.take().unwrap();
    let out = thread::spawn(move || drain(stdout, stdout_log));
    let err = thread::spawn(move || drain(stderr, stderr_log));
    let mut timed_out = false;
    let mut cancelled = false;
    let status = loop {
        if let Some(s) = child.try_wait()? {
            break s;
        }
        if cancel
            .as_ref()
            .is_some_and(|c| c.load(std::sync::atomic::Ordering::SeqCst))
        {
            cancelled = true;
            unsafe {
                libc::kill(-(child.id() as i32), libc::SIGKILL);
            }
            break child.wait()?;
        }
        if begin.elapsed() >= timeout {
            timed_out = true;
            unsafe {
                libc::kill(-(child.id() as i32), libc::SIGKILL);
            }
            break child.wait()?;
        }
        thread::sleep(Duration::from_millis(10));
    };
    // Reap/stop inherited pipes from descendants in the same process group even
    // after the parent exits. This is a timeout mechanism, not containment.
    unsafe {
        libc::kill(-(child.id() as i32), libc::SIGKILL);
    }
    let (a, ta) = out
        .join()
        .map_err(|_| anyhow::anyhow!("stdout reader failed"))?;
    let (b, tb) = err
        .join()
        .map_err(|_| anyhow::anyhow!("stderr reader failed"))?;
    let output = format!(
        "{}{}",
        String::from_utf8_lossy(&a),
        String::from_utf8_lossy(&b)
    );
    Ok(CheckResult {
        check: check.id.clone(),
        language: language.into(),
        path: path.into(),
        status: if cancelled {
            "cancelled"
        } else if timed_out {
            "timeout"
        } else if status.success() {
            "passed"
        } else {
            "failed"
        }
        .into(),
        command,
        exit_code: status.code(),
        seconds: begin.elapsed().as_secs_f64(),
        output,
        output_truncated: ta || tb,
    })
}
#[cfg(not(unix))]
pub fn run(
    _root: &Path,
    _check: &Check,
    _language: &str,
    _path: &str,
    _timeout: Duration,
) -> Result<CheckResult> {
    bail!("command runner currently requires Unix process groups")
}
pub fn check(
    root: &Path,
    registry_path: &Path,
    timeout: u64,
    selected: &[String],
) -> Result<Report> {
    if !(1..=3600).contains(&timeout) {
        bail!("timeout must be 1..3600 seconds")
    }
    let (inventory, r) = scan(root, registry_path)?;
    if selected
        .iter()
        .any(|id| !r.languages.iter().any(|l| &l.id == id))
    {
        bail!("unknown selected language")
    }
    let root = PathBuf::from(&inventory.root);
    let mut results = Vec::new();
    let mut project_done = BTreeSet::new();
    for file in &inventory.files {
        if !selected.is_empty() && !selected.contains(&file.language) {
            continue;
        }
        let l = r.languages.iter().find(|l| l.id == file.language).unwrap();
        if l.checks.is_empty() {
            results.push(CheckResult {
                check: "no-checks".into(),
                language: l.id.clone(),
                path: file.path.clone(),
                status: "not_configured".into(),
                command: vec![],
                exit_code: None,
                seconds: 0.,
                output: "no checks configured for this language".into(),
                output_truncated: false,
            });
        }
        for c in &l.checks {
            if let Some(marker) = &c.project_marker {
                if !project_done.insert(format!("{}:{}", l.id, c.id)) {
                    continue;
                }
                if !root.join(marker).is_file() {
                    results.push(CheckResult {
                        check: c.id.clone(),
                        language: l.id.clone(),
                        path: marker.clone(),
                        status: "not_configured".into(),
                        command: vec![],
                        exit_code: None,
                        seconds: 0.,
                        output:
                            "project marker absent at scan root; check each project root separately"
                                .into(),
                        output_truncated: false,
                    });
                    continue;
                }
            }
            if c.program == "@json" || c.program == "@yaml" {
                let bytes = fs::read(root.join(&file.path))?;
                let parsed = if c.program == "@json" {
                    serde_json::from_slice::<serde_json::Value>(&bytes)
                        .map(|_| ())
                        .map_err(|e| e.to_string())
                } else {
                    serde_yaml::from_slice::<serde_yaml::Value>(&bytes)
                        .map(|_| ())
                        .map_err(|e| e.to_string())
                };
                results.push(CheckResult {
                    check: c.id.clone(),
                    language: l.id.clone(),
                    path: file.path.clone(),
                    status: if parsed.is_ok() { "passed" } else { "failed" }.into(),
                    command: vec![c.program.clone()],
                    exit_code: None,
                    seconds: 0.,
                    output: parsed.err().unwrap_or_default(),
                    output_truncated: false,
                });
            } else {
                results.push(run(
                    &root,
                    c,
                    &l.id,
                    &file.path,
                    Duration::from_secs(timeout),
                )?)
            }
        }
    }
    let mut summary = BTreeMap::new();
    for r in &results {
        *summary.entry(r.status.clone()).or_default() += 1;
    }
    let complete = !results.is_empty()
        && inventory.ignored_large.is_empty()
        && results.iter().all(|r| r.status == "passed");
    Ok(Report {
        schema_version: 1,
        inventory,
        checks: results,
        summary,
        complete,
        execution_mode: "trusted_local_no_sandbox".into(),
        selected_languages: selected.to_vec(),
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn scanner_custom_language_and_symlink() {
        let d = tempfile::tempdir().unwrap();
        fs::write(d.path().join("x.xyz"), "content").unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(d.path().join("x.xyz"), d.path().join("link.xyz")).unwrap();
        fs::create_dir(d.path().join("target")).unwrap();
        fs::write(d.path().join("target/ignored.xyz"), "ignored").unwrap();
        let config = d.path().join("registry.json");
        fs::write(&config,r#"{"version":1,"languages":[{"id":"custom","extensions":["xyz"],"filenames":[],"checks":[] }]}"#).unwrap();
        let (i, _) = scan(d.path(), &config).unwrap();
        assert_eq!(i.files.len(), 1);
        assert_eq!(i.files[0].language, "custom");
    }
    #[test]
    fn invalid_json_is_failure() {
        let d = tempfile::tempdir().unwrap();
        fs::write(d.path().join("bad.json"), "{").unwrap();
        let config = d.path().join("r.cfg");
        fs::write(&config,r#"{"version":1,"languages":[{"id":"json","extensions":["json"],"filenames":[],"checks":[{"id":"parse","kind":"syntax","program":"@json","args":[]}]}]}"#).unwrap();
        let r = check(d.path(), &config, 1, &[]).unwrap();
        assert!(!r.complete);
        assert_eq!(r.checks[0].status, "failed");
    }
    #[cfg(unix)]
    #[test]
    fn missing_tool_not_pass() {
        let d = tempfile::tempdir().unwrap();
        let c = Check {
            id: "missing".into(),
            kind: "lint".into(),
            program: "aegis-nonexistent-tool-abc".into(),
            args: vec![],
            project_marker: None,
        };
        assert_eq!(
            run(d.path(), &c, "x", "x", Duration::from_secs(1))
                .unwrap()
                .status,
            "unavailable"
        );
    }
    #[cfg(unix)]
    #[test]
    fn arguments_are_not_shell_interpolated() {
        let d = tempfile::tempdir().unwrap();
        let c = Check {
            id: "literal".into(),
            kind: "syntax".into(),
            program: "echo".into(),
            args: vec!["{file}".into()],
            project_marker: None,
        };
        let r = run(
            d.path(),
            &c,
            "x",
            "$(touch PWNED).py",
            Duration::from_secs(1),
        )
        .unwrap();
        assert_eq!(r.status, "passed");
        assert!(r.output.contains("$(touch PWNED)"));
        assert!(!d.path().join("PWNED").exists());
    }
    #[cfg(unix)]
    #[test]
    fn timeout_kills_group() {
        let d = tempfile::tempdir().unwrap();
        let c = Check {
            id: "timeout".into(),
            kind: "test".into(),
            program: "sleep".into(),
            args: vec!["10".into()],
            project_marker: None,
        };
        let r = run(d.path(), &c, "x", "x", Duration::from_millis(40)).unwrap();
        assert_eq!(r.status, "timeout");
        assert!(r.seconds < 2.);
    }
}

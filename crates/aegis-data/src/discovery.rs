//! Bounded, non-executing repository discovery and reviewable plan generation.
use crate::operator::{Plan, Stage};
use anyhow::{bail, Result};
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::Path,
};
fn stage(
    id: String,
    program: &str,
    args: Vec<String>,
    inputs: &[String],
    dependency: Option<String>,
) -> Stage {
    Stage {
        id,
        operation: "check".into(),
        program: program.into(),
        args,
        depends_on: dependency.into_iter().collect(),
        inputs: inputs.to_vec(),
        outputs: vec![],
        slots: 1,
        max_attempts: 1,
        retry_exit_codes: vec![],
        resume_args: vec![],
        container: None,
        timeout_seconds: 300,
        metric_log: None,
    }
}
pub fn discover(workspace: &Path, run_id: &str) -> Result<Value> {
    let root = workspace.canonicalize()?;
    if !root.is_dir() {
        bail!("workspace must be a directory")
    }
    let mut inputs = Vec::new();
    let mut manifests = Vec::new();
    let mut languages = BTreeMap::<String, usize>::new();
    let mut skipped = 0;
    let mut visited = 0;
    let mut bytes = 0u64;
    for entry in ignore::WalkBuilder::new(&root)
        .max_depth(Some(8))
        .hidden(false)
        .follow_links(false)
        .filter_entry(|e| {
            !e.file_type().is_some_and(|t| t.is_dir())
                || (!e.file_name().to_string_lossy().starts_with(".aegis-build-")
                    && ![
                        ".git",
                        "target",
                        "node_modules",
                        ".venv",
                        "venv",
                        "build",
                        "dist",
                        "__pycache__",
                        ".aegis",
                    ]
                    .contains(&e.file_name().to_string_lossy().as_ref()))
        })
        .build()
    {
        let entry = entry?;
        visited += 1;
        if visited > 20000 {
            bail!("discovery directory-entry budget exceeded; choose a smaller root")
        }
        if !entry.file_type().is_some_and(|t| t.is_file()) {
            continue;
        }
        if inputs.len() + skipped >= 10000 {
            bail!("discovery file budget exceeded; choose a smaller project root")
        }
        let filename = entry.file_name().to_string_lossy();
        if filename.starts_with(".env")
            || ["id_rsa", "id_ed25519"].contains(&filename.as_ref())
            || ["pem", "key", "p12"]
                .iter()
                .any(|extension| entry.path().extension().is_some_and(|e| e == *extension))
        {
            skipped += 1;
            continue;
        }
        let path = entry.path();
        let relative = path.strip_prefix(&root)?.to_string_lossy().into_owned();
        let size = entry.metadata()?.len();
        if size > 1024 * 1024 {
            skipped += 1;
            continue;
        }
        bytes += size;
        if bytes > 64 * 1024 * 1024 {
            bail!("discovery source budget exceeded")
        }
        let name = entry.file_name().to_string_lossy();
        if [
            "Cargo.toml",
            "go.mod",
            "package.json",
            "pyproject.toml",
            "CMakeLists.txt",
        ]
        .contains(&name.as_ref())
        {
            manifests.push(relative.clone());
        }
        if let Some(extension) = path.extension().and_then(|s| s.to_str()) {
            let language = match extension {
                "rs" => "rust",
                "py" => "python",
                "js" | "jsx" => "javascript",
                "ts" | "tsx" => "typescript",
                "go" => "go",
                "c" | "h" => "c",
                "cpp" | "hpp" | "cc" => "cpp",
                "java" => "java",
                "cs" => "csharp",
                "rb" => "ruby",
                "sh" => "shell",
                _ => "other",
            };
            *languages.entry(language.into()).or_default() += 1;
        }
        inputs.push(relative);
    }
    inputs.sort();
    manifests.sort();
    if manifests.len() > 32 {
        bail!("too many project manifests; select a subproject")
    }
    let input_bytes = serde_json::to_vec(&inputs)?.len();
    let possible_stages = manifests.len().saturating_mul(3).max(1);
    if input_bytes.saturating_mul(possible_stages) > 512 * 1024 {
        bail!("discovery plan would exceed interface budget; choose a smaller project root")
    }
    let mut stages = Vec::new();
    let mut roots = Vec::new();
    let mut warnings=vec!["Generated commands require review: repository build/test scripts execute project code. No dependency installation or execution occurred.".to_string(),"Discovery is bounded to depth 8, 10,000 files, 1 MiB/file and 64 MiB total. Skipped/generated/ignored files are outside input identity coverage.".to_string(),"Generated plans do not declare build artifacts; use a fresh run ID for every execution. Add explicit outputs for stronger artifact-based reuse/export.".to_string()];
    let mut selected = BTreeSet::new();
    for manifest in &manifests {
        let parent = Path::new(manifest).parent().unwrap();
        let directory = if parent.as_os_str().is_empty() {
            ".".to_string()
        } else {
            parent.to_string_lossy().into_owned()
        };
        let name = Path::new(manifest).file_name().unwrap().to_string_lossy();
        // A Cargo workspace's test command covers its members; skip nested Cargo manifests.
        if name == "Cargo.toml"
            && selected
                .iter()
                .any(|p: &String| Path::new(manifest).starts_with(p) && p != manifest)
        {
            continue;
        }
        let before = stages.len();
        let mut add = |program: &str, args: Vec<String>| {
            let id = format!("check-{}", stages.len() + 1);
            let dependency = stages.last().map(|s: &Stage| s.id.clone());
            stages.push(stage(id, program, args, &inputs, dependency));
        };
        match name.as_ref() {
            "Cargo.toml" => {
                add(
                    "cargo",
                    vec![
                        "test".into(),
                        "--locked".into(),
                        "--manifest-path".into(),
                        manifest.clone(),
                    ],
                );
                if !root.join(parent).join("Cargo.lock").is_file() {
                    warnings.push(format!("{manifest}: no Cargo.lock detected here; --locked may require creating/reviewing a lockfile first"));
                }
                let source = fs::read_to_string(root.join(manifest))?;
                if source.lines().any(|line| line.trim() == "[workspace]") {
                    selected.insert(if directory == "." {
                        "".into()
                    } else {
                        format!("{directory}/")
                    });
                }
            }
            "go.mod" => add(
                "go",
                vec![
                    "-C".into(),
                    directory.clone(),
                    "test".into(),
                    "./...".into(),
                ],
            ),
            "package.json" => {
                let package: Value = serde_json::from_slice(&fs::read(root.join(manifest))?)?;
                for script in ["build", "test"] {
                    if package["scripts"][script].is_string() {
                        add(
                            "npm",
                            vec![
                                "--prefix".into(),
                                directory.clone(),
                                "run".into(),
                                script.into(),
                            ],
                        );
                    }
                }
                if !root.join(parent).join("package-lock.json").is_file() {
                    warnings.push(format!("{manifest}: no package-lock.json detected; dependency versions need review"));
                }
            }
            "pyproject.toml" => {
                let tests = root.join(parent).join("tests");
                if tests.is_dir() {
                    add(
                        "python3",
                        vec!["-m".into(), "pytest".into(), format!("{directory}/tests")],
                    );
                    warnings.push(format!("{manifest}: pytest is a proposed test runner; its module/dependency availability must be verified"));
                } else {
                    warnings.push(format!(
                        "{manifest}: no tests/ directory; no test command guessed"
                    ));
                }
            }
            "CMakeLists.txt" => {
                let build = format!(".aegis-build-{}", before + 1);
                add(
                    "cmake",
                    vec!["-S".into(), directory.clone(), "-B".into(), build.clone()],
                );
                add(
                    "cmake",
                    vec![
                        "--build".into(),
                        build.clone(),
                        "--parallel".into(),
                        "2".into(),
                    ],
                );
                add(
                    "ctest",
                    vec!["--test-dir".into(), build, "--output-on-failure".into()],
                );
            }
            _ => {}
        }
        roots.push(json!({"directory":directory,"manifest":manifest,"proposed_stages":stages.len()-before}));
    }
    let plan = Plan {
        run_id: run_id.into(),
        workspace: root.clone(),
        mode: "bounded".into(),
        max_slots: 1,
        max_wall_seconds: 3600,
        allowed_operations: vec!["check".into()],
        stages,
    };
    if !plan.stages.is_empty() {
        plan.validate()?;
    }
    if serde_json::to_vec(&plan)?.len() > 768 * 1024 {
        bail!("generated plan exceeds API budget; choose a smaller project root")
    }
    let programs: BTreeSet<String> = plan.stages.iter().map(|s| s.program.clone()).collect();
    let tools:Vec<_>=programs.iter().map(|p|json!({"program":p,"available":crate::operator::executable_hash(&root,p).is_some()})).collect();
    let profile = json!({"schema_version":1,"id":"discovered-host-review","require_container":false,"allowed_programs":programs,
        "allowed_images":[],"max_slots":1,"max_wall_seconds":3600,"max_attempts":1,"max_stage_seconds":300,"max_cpu":1,"max_memory_mib":128});
    Ok(
        json!({"schema_version":1,"workspace":root,"languages":languages,"project_roots":roots,"tools":tools,
        "skipped_files":skipped,"input_files":inputs.len(),"plan":plan,"profile":profile,"warnings":warnings,
        "runnable_plan_generated":!programs.is_empty(),"execution":"not_started","dependency_installation":"not_performed"}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn npm_scripts_are_proposed_without_execution_and_secrets_are_excluded() {
        let root = tempfile::tempdir().unwrap();
        fs::write(
            root.path().join("package.json"),
            r#"{"scripts":{"test":"touch should-not-exist"}}"#,
        )
        .unwrap();
        fs::write(root.path().join(".env"), "SECRET=not-an-input").unwrap();
        let report = discover(root.path(), "review").unwrap();
        assert_eq!(report["plan"]["stages"][0]["program"], "npm");
        assert_eq!(report["input_files"], 1);
        assert!(!root.path().join("should-not-exist").exists());
        assert!(discover(root.path(), "../escape").is_err());
    }
    #[test]
    fn cargo_workspace_and_generated_tree_do_not_duplicate_checks() {
        let root = tempfile::tempdir().unwrap();
        fs::write(
            root.path().join("Cargo.toml"),
            "[workspace]\nmembers=['member']\n",
        )
        .unwrap();
        fs::create_dir(root.path().join("member")).unwrap();
        fs::write(
            root.path().join("member/Cargo.toml"),
            "[package]\nname='member'\n",
        )
        .unwrap();
        fs::create_dir(root.path().join(".aegis-build-1")).unwrap();
        fs::write(
            root.path().join(".aegis-build-1/CMakeLists.txt"),
            "generated",
        )
        .unwrap();
        let report = discover(root.path(), "review").unwrap();
        assert_eq!(report["plan"]["stages"].as_array().unwrap().len(), 1);
        assert_eq!(report["input_files"], 2);
    }
}

use aegis_data::{ingest, ledger::Ledger, metrics};
use anyhow::{bail, Result};
use clap::{Parser, Subcommand};
use serde::Deserialize;
use std::{fs, path::PathBuf};
#[derive(Parser)]
#[command(version, about = "AEGIS native data and evaluation tools")]
struct Cli {
    #[command(subcommand)]
    cmd: Command,
}
#[derive(Subcommand)]
enum Command {
    UseCases,
    /// Discover implemented agent interfaces and unavailable integrations.
    Capabilities,
    WirePack {
        source: PathBuf,
        out: PathBuf,
        #[arg(long, default_value = "raw")]
        encoding: String,
        #[arg(long, default_value = "artifact")]
        topic: String,
    },
    WireUnpack {
        source: PathBuf,
        out: PathBuf,
    },
    /// Run a local model protocol adapter; write live events and a final receipt.
    ModelRun {
        workspace: PathBuf,
        adapter: PathBuf,
        request: PathBuf,
        out: PathBuf,
        #[arg(long)]
        trusted_local: bool,
    },
    /// Run a bounded, read-only local language-server query.
    LanguageQuery {
        workspace: PathBuf,
        server: PathBuf,
        query: PathBuf,
        out: PathBuf,
        #[arg(long)]
        trusted_local: bool,
    },
    /// Build an optional lexical document index from a provenance-bearing JSON array.
    KnowledgeBuild {
        documents: PathBuf,
        out: PathBuf,
    },
    /// Assemble cited context; omit index in request JSON to disable retrieval.
    KnowledgeContext {
        request: PathBuf,
        out: PathBuf,
    },
    /// Export only caller-approved conversations; never starts training.
    ChatExport {
        conversations: PathBuf,
        out: PathBuf,
    },
    EditorTree {
        workspace: PathBuf,
    },
    EditorRead {
        workspace: PathBuf,
        path: String,
    },
    EditorSave {
        workspace: PathBuf,
        path: String,
        replacement: PathBuf,
        #[arg(long)]
        expected_sha256: String,
        #[arg(long)]
        trusted_local: bool,
    },
    EditorRollback {
        workspace: PathBuf,
        path: String,
        snapshot: String,
        #[arg(long)]
        expected_sha256: String,
        #[arg(long)]
        trusted_local: bool,
    },
    ProjectDiscover {
        workspace: PathBuf,
        out: PathBuf,
        #[arg(long)]
        run_id: String,
    },
    ArtifactPack {
        source: PathBuf,
        store: PathBuf,
        manifest: PathBuf,
        #[arg(long, default_value_t = 8)]
        chunk_mib: u64,
    },
    ArtifactTransfer {
        manifest: PathBuf,
        source_store: PathBuf,
        destination_store: PathBuf,
        #[arg(long)]
        manifest_sha256: String,
    },
    ArtifactMaterialize {
        manifest: PathBuf,
        store: PathBuf,
        out: PathBuf,
        #[arg(long)]
        manifest_sha256: String,
    },
    OutcomeReport {
        input: PathBuf,
        out: PathBuf,
    },
    ProjectVerify {
        package: PathBuf,
        #[arg(long)]
        manifest_sha256: String,
    },
    ProjectDoctor {
        plan: PathBuf,
    },
    ProjectExport {
        db: String,
        run_id: String,
        out: PathBuf,
    },
    ProjectRun {
        profile: PathBuf,
        plan: PathBuf,
        db: String,
        out: PathBuf,
        #[arg(long)]
        trusted_local: bool,
    },
    RuntimeCheck {
        profile: PathBuf,
        plan: PathBuf,
    },
    RuntimeRun {
        profile: PathBuf,
        plan: PathBuf,
        db: String,
        #[arg(long)]
        trusted_local: bool,
    },
    CaseAcceptRemote {
        case: PathBuf,
        db: String,
        job: String,
    },
    LabRun {
        dataset: PathBuf,
        adapter: PathBuf,
        out: PathBuf,
        #[arg(long)]
        trusted_local: bool,
    },
    LabCompare {
        dataset: PathBuf,
        baseline: PathBuf,
        candidate: PathBuf,
        #[arg(long, default_value = "exact")]
        mode: String,
        #[arg(long)]
        out: PathBuf,
    },
    LineageLink {
        db: String,
        output: PathBuf,
        inputs: Vec<PathBuf>,
    },
    LineageQuarantine {
        db: String,
        sha256: String,
        reason: String,
    },
    LineageCheck {
        db: String,
        sha256: String,
    },
    BrokerQuote {
        request: PathBuf,
        inventory: PathBuf,
        #[arg(long)]
        out: PathBuf,
    },
    IncidentDrill {
        workspace: PathBuf,
        binary: PathBuf,
        out: PathBuf,
        #[arg(long)]
        scenario: String,
        #[arg(long, default_value = "python3")]
        python: String,
        #[arg(long)]
        trusted_local: bool,
    },
    CaseCapture {
        source: PathBuf,
        spec: PathBuf,
        out: PathBuf,
    },
    CasePropose {
        case: PathBuf,
        generator: PathBuf,
        #[arg(long)]
        trusted_local: bool,
    },
    CaseRehearse {
        case: PathBuf,
        patch: PathBuf,
        #[arg(long)]
        trusted_local: bool,
    },
    CaseExport {
        case: PathBuf,
        out: PathBuf,
    },
    CaseReplay {
        package: PathBuf,
        out: PathBuf,
        #[arg(long)]
        exchange_sha256: String,
        #[arg(long)]
        trusted_local: bool,
    },
    CasePromote {
        case: PathBuf,
        source: PathBuf,
        #[arg(long)]
        report_sha256: String,
        #[arg(long)]
        rollback: bool,
        #[arg(long)]
        remote_db: Option<String>,
        #[arg(long)]
        remote_job: Option<String>,
        #[arg(long)]
        lineage_db: Option<String>,
    },
    Readiness {
        registry: PathBuf,
        #[arg(long, default_value = ".")]
        root: PathBuf,
        #[arg(long)]
        out: Option<PathBuf>,
    },
    WorkerController {
        db: String,
        #[arg(long)]
        tokens: PathBuf,
        #[arg(long, default_value = "127.0.0.1:8790")]
        bind: std::net::SocketAddr,
        #[arg(long, default_value_t = 30)]
        ttl: i64,
    },
    WorkerRemote {
        controller: String,
        worker: String,
        root: PathBuf,
        #[arg(long, default_value = "AEGIS_WORKER_TOKEN")]
        token_env: String,
        #[arg(long)]
        trusted_local: bool,
    },
    RemoteVerify {
        db: String,
        id: String,
    },
    WorkerEnqueue {
        db: String,
        plan: PathBuf,
        bundle: PathBuf,
        #[arg(long)]
        worker: Option<String>,
    },
    WorkerOnce {
        db: String,
        worker: String,
        #[arg(long, default_value_t = 15)]
        ttl: i64,
        #[arg(long)]
        trusted_local: bool,
    },
    WorkerVerify {
        db: String,
        id: String,
    },
    WorkerStatus {
        db: String,
        id: String,
    },
    GraphExtract {
        source: PathBuf,
        #[arg(long)]
        source_url: String,
        #[arg(long)]
        out: PathBuf,
    },
    OperatorNote {
        db: String,
        run_id: String,
        message: String,
        #[arg(long, default_value = "human-cli")]
        actor: String,
    },
    OperatorReference {
        db: String,
        corpus: PathBuf,
        #[arg(long)]
        out: String,
        #[arg(long)]
        run_id: String,
        #[arg(long, default_value_t = 20)]
        steps: usize,
        #[arg(long, default_value = "python3")]
        python: String,
        #[arg(long)]
        trusted_local: bool,
        #[arg(long)]
        plan_only: bool,
    },
    GraphDiff {
        db: String,
        before: i64,
        after: i64,
    },
    RepairPropose {
        source: PathBuf,
        out: PathBuf,
        #[arg(long, default_value = "ruff-format")]
        tool: String,
    },
    RepairApply {
        source: PathBuf,
        receipt: PathBuf,
        #[arg(long)]
        rollback: bool,
    },
    OperatorServe {
        db: String,
        workspace: PathBuf,
        #[arg(long, default_value = "127.0.0.1:8787")]
        bind: std::net::SocketAddr,
        #[arg(long)]
        trusted_local: bool,
    },
    OperatorRun {
        db: String,
        plan: PathBuf,
        #[arg(long)]
        trusted_local: bool,
    },
    OperatorStatus {
        db: String,
        run_id: String,
    },
    OperatorControl {
        db: String,
        run_id: String,
        action: String,
        #[arg(long, default_value = "human-cli")]
        actor: String,
    },
    OperatorEvents {
        db: String,
        run_id: String,
        #[arg(long, default_value_t = 0)]
        after: i64,
    },
    GraphSnapshot {
        db: String,
        #[arg(long)]
        at: Option<i64>,
        #[arg(long)]
        out: PathBuf,
    },
    GraphImpact {
        db: String,
        id: String,
    },
    GraphVerify {
        db: String,
    },
    GraphImportCatalog {
        db: String,
        #[arg(long, default_value = "graphs/catalog.json")]
        catalog: PathBuf,
    },
    Renew {
        db: String,
        id: String,
        worker: String,
        epoch: i64,
        #[arg(long, default_value_t = 300)]
        ttl: i64,
    },
    Cancel {
        db: String,
        id: String,
    },
    ValidateDataset {
        root: PathBuf,
    },
    Compare {
        baseline: PathBuf,
        candidate: PathBuf,
        #[arg(long)]
        out: PathBuf,
    },
    CodeScan {
        root: PathBuf,
        #[arg(long, default_value = "configs/languages.json")]
        registry: PathBuf,
        #[arg(long)]
        out: PathBuf,
    },
    CodeCheck {
        root: PathBuf,
        #[arg(long, default_value = "configs/languages.json")]
        registry: PathBuf,
        #[arg(long)]
        out: PathBuf,
        #[arg(long, default_value_t = 120)]
        timeout: u64,
        #[arg(long)]
        trusted_local: bool,
        #[arg(long, value_delimiter = ',')]
        languages: Vec<String>,
    },
    Tokenize {
        input: PathBuf,
        out: PathBuf,
        #[arg(long, default_value_t = 16)]
        tokens: usize,
        #[arg(long, default_value_t = 4096)]
        max_rows: usize,
    },
    Prepare {
        input: PathBuf,
        out: PathBuf,
        #[arg(long, default_value_t = 3)]
        prefetch: usize,
    },
    Inspect {
        input: PathBuf,
    },
    Classify {
        input: PathBuf,
    },
    Regression {
        input: PathBuf,
    },
    Gate {
        input: PathBuf,
        #[arg(long)]
        out: Option<PathBuf>,
    },
    Plan {
        input: PathBuf,
    },
    Enqueue {
        db: String,
        id: String,
        payload: String,
    },
    Claim {
        db: String,
        worker: String,
        #[arg(long, default_value_t = 300)]
        ttl: i64,
    },
    Complete {
        db: String,
        id: String,
        worker: String,
        epoch: i64,
        result: String,
    },
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Cluster {
    nodes: Vec<Node>,
    shards: Vec<String>,
    prefetch: usize,
    max_tokens: usize,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Node {
    id: String,
}
fn now() -> Result<i64> {
    Ok(i64::try_from(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_secs(),
    )?)
}
#[tokio::main]
async fn main() -> Result<()> {
    match Cli::parse().cmd {
        Command::EditorTree { workspace } => println!("{}", aegis_data::editor::tree(&workspace)?),
        Command::EditorRead { workspace, path } => {
            println!("{}", aegis_data::editor::read(&workspace, &path)?)
        }
        Command::EditorSave {
            workspace,
            path,
            replacement,
            expected_sha256,
            trusted_local,
        } => {
            if !trusted_local {
                bail!("--trusted-local required for editor writes")
            }
            if fs::metadata(&replacement)?.len() > 128 * 1024 {
                bail!("replacement exceeds editor limit")
            }
            println!(
                "{}",
                aegis_data::editor::save(
                    &workspace,
                    &path,
                    &expected_sha256,
                    &fs::read_to_string(replacement)?
                )?
            );
        }
        Command::EditorRollback {
            workspace,
            path,
            snapshot,
            expected_sha256,
            trusted_local,
        } => {
            if !trusted_local {
                bail!("--trusted-local required for editor rollback")
            }
            println!(
                "{}",
                aegis_data::editor::rollback(&workspace, &path, &expected_sha256, &snapshot)?
            );
        }
        Command::ProjectDiscover {
            workspace,
            out,
            run_id,
        } => {
            let report = aegis_data::discovery::discover(&workspace, &run_id)?;
            if report["runnable_plan_generated"] != true {
                println!("{report}");
                bail!("no supported build/test plan detected")
            }
            fs::create_dir(&out)?;
            fs::write(
                out.join("plan.json"),
                serde_json::to_vec_pretty(&report["plan"])?,
            )?;
            fs::write(
                out.join("profile.json"),
                serde_json::to_vec_pretty(&report["profile"])?,
            )?;
            fs::write(
                out.join("discovery.json"),
                serde_json::to_vec_pretty(&report)?,
            )?;
            println!("{report}");
        }
        Command::ArtifactPack {
            source,
            store,
            manifest,
            chunk_mib,
        } => println!(
            "{}",
            aegis_data::artifact::pack(&source, &store, &manifest, chunk_mib)?
        ),
        Command::ArtifactTransfer {
            manifest,
            source_store,
            destination_store,
            manifest_sha256,
        } => println!(
            "{}",
            aegis_data::artifact::transfer(
                &manifest,
                &source_store,
                &destination_store,
                &manifest_sha256
            )?
        ),
        Command::ArtifactMaterialize {
            manifest,
            store,
            out,
            manifest_sha256,
        } => println!(
            "{}",
            aegis_data::artifact::materialize(&manifest, &store, &out, &manifest_sha256)?
        ),
        Command::OutcomeReport { input, out } => {
            let report = aegis_data::outcomes::report(&input)?;
            fs::write(out, serde_json::to_vec_pretty(&report)?)?;
            println!("{report}");
        }
        Command::ProjectVerify {
            package,
            manifest_sha256,
        } => println!(
            "{}",
            aegis_data::project::verify_export(&package, &manifest_sha256)?
        ),
        Command::ProjectDoctor { plan } => {
            let plan = serde_json::from_slice(&fs::read(plan)?)?;
            let report = aegis_data::project::doctor(&plan)?;
            println!("{report}");
            if report["ready"] != true {
                bail!("project tools unavailable")
            }
        }
        Command::ProjectExport { db, run_id, out } => {
            println!("{}", aegis_data::project::export(&db, &run_id, &out)?)
        }
        Command::ProjectRun {
            profile,
            plan,
            db,
            out,
            trusted_local,
        } => {
            let profile: aegis_data::runtime::Profile =
                serde_json::from_slice(&fs::read(profile)?)?;
            let plan: aegis_data::operator::Plan = serde_json::from_slice(&fs::read(plan)?)?;
            profile.admit(&plan)?;
            let run = plan.run_id.clone();
            let result = aegis_data::operator::run(&db, plan, trusted_local).await?;
            if result["status"] != "completed" {
                bail!("project failed; inspect operator-status and logs; export withheld")
            }
            println!("{}", aegis_data::project::export(&db, &run, &out)?);
        }
        Command::RuntimeCheck { profile, plan } => {
            let profile: aegis_data::runtime::Profile =
                serde_json::from_slice(&fs::read(profile)?)?;
            let plan: aegis_data::operator::Plan = serde_json::from_slice(&fs::read(plan)?)?;
            println!("{}", profile.admit(&plan)?);
        }
        Command::RuntimeRun {
            profile,
            plan,
            db,
            trusted_local,
        } => {
            let profile: aegis_data::runtime::Profile =
                serde_json::from_slice(&fs::read(profile)?)?;
            let plan: aegis_data::operator::Plan = serde_json::from_slice(&fs::read(plan)?)?;
            println!("{}", profile.admit(&plan)?);
            println!(
                "{}",
                aegis_data::operator::run(&db, plan, trusted_local).await?
            );
        }
        Command::KnowledgeBuild { documents, out } => {
            let docs = aegis_data::knowledge::read_json(&documents)?;
            let index = aegis_data::knowledge::build(docs)?;
            aegis_data::knowledge::publish(&out, &serde_json::to_vec_pretty(&index)?)?;
            println!(
                "{}",
                serde_json::json!({"chunks_sha256":index.chunks_sha256,"chunks":index.chunks.len()})
            );
        }
        Command::KnowledgeContext { request, out } => {
            let result =
                aegis_data::knowledge::context(aegis_data::knowledge::read_json(&request)?)?;
            aegis_data::knowledge::publish(&out, &serde_json::to_vec_pretty(&result)?)?;
        }
        Command::ChatExport { conversations, out } => {
            let bytes =
                aegis_data::knowledge::export(aegis_data::knowledge::read_json(&conversations)?)?;
            aegis_data::knowledge::publish(&out, &bytes)?;
            println!(
                "{}",
                serde_json::json!({"sha256":ingest::hash(&bytes),"bytes":bytes.len(),"training_started":false})
            );
        }
        Command::LanguageQuery {
            workspace,
            server,
            query,
            out,
            trusted_local,
        } => {
            if !trusted_local {
                bail!("--trusted-local required for language-server execution")
            }
            let result = aegis_data::language::query(
                &workspace,
                &aegis_data::knowledge::read_json(&server)?,
                aegis_data::knowledge::read_json(&query)?,
            )?;
            aegis_data::knowledge::publish(&out, &serde_json::to_vec_pretty(&result)?)?;
        }
        Command::ModelRun {
            workspace,
            adapter,
            request,
            out,
            trusted_local,
        } => {
            use std::io::Write;
            if !trusted_local {
                bail!("--trusted-local required for model adapter execution")
            }
            let adapter = aegis_data::knowledge::read_json(&adapter)?;
            let request = aegis_data::knowledge::read_json(&request)?;
            fs::create_dir(&out)?;
            let mut events = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(out.join("events.jsonl"))?;
            let result = aegis_data::model::generate(
                &workspace.canonicalize()?,
                &adapter,
                request,
                std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
                |event| {
                    serde_json::to_writer(&mut events, &event)?;
                    events.write_all(b"\n")?;
                    events.flush()?;
                    Ok(())
                },
            );
            match result {
                Ok(receipt) => {
                    events.sync_all()?;
                    aegis_data::knowledge::publish(
                        &out.join("receipt.json"),
                        &serde_json::to_vec_pretty(&receipt)?,
                    )?;
                }
                Err(error) => {
                    serde_json::to_writer(
                        &mut events,
                        &serde_json::json!({"type":"failed","error":error.to_string()}),
                    )?;
                    events.write_all(b"\n")?;
                    events.sync_all()?;
                    return Err(error);
                }
            }
        }
        Command::WirePack {
            source,
            out,
            encoding,
            topic,
        } => {
            use aegis_data::packet::{Encoding, Topic};
            let encoding = match encoding.as_str() {
                "raw" => Encoding::Raw,
                "protobuf" => Encoding::Protobuf,
                "flatbuffers" => Encoding::Flatbuffers,
                _ => bail!("unsupported encoding"),
            };
            let topic = match topic.as_str() {
                "artifact" => Topic::Artifact,
                "training-batch" => Topic::TrainingBatch,
                "telemetry" => Topic::Telemetry,
                "tool-event" => Topic::ToolEvent,
                _ => bail!("unsupported topic"),
            };
            let frame = aegis_data::packet::pack(
                aegis_data::packet::read(&source, false)?,
                encoding,
                topic,
            )?;
            aegis_data::knowledge::publish(&out, &frame)?;
            println!(
                "{}",
                aegis_data::packet::summary(&aegis_data::packet::unpack(&frame)?)
            );
        }
        Command::WireUnpack { source, out } => {
            let packet = aegis_data::packet::unpack(&aegis_data::packet::read(&source, true)?)?;
            aegis_data::knowledge::publish(&out, &packet.payload)?;
            println!("{}", aegis_data::packet::summary(&packet));
        }
        Command::Capabilities => println!(
            "{}",
            serde_json::to_string_pretty(&aegis_data::capabilities::catalog())?
        ),
        Command::UseCases => println!(
            "{}",
            serde_json::to_string_pretty(&aegis_data::uses::catalog())?
        ),
        Command::CaseAcceptRemote { case, db, job } => {
            println!("{}", aegis_data::cases::accept_remote(&case, &db, &job)?)
        }
        Command::LabRun {
            dataset,
            adapter,
            out,
            trusted_local,
        } => println!(
            "{}",
            aegis_data::lab::run(&dataset, &adapter, &out, trusted_local)?
        ),
        Command::LabCompare {
            dataset,
            baseline,
            candidate,
            mode,
            out,
        } => {
            let r = aegis_data::lab::compare(&dataset, &baseline, &candidate, &mode)?;
            fs::write(out, serde_json::to_vec_pretty(&r)?)?;
            println!("{r}")
        }
        Command::LineageLink { db, output, inputs } => {
            println!("{}", aegis_data::uses::link(&db, &output, &inputs)?)
        }
        Command::LineageQuarantine { db, sha256, reason } => {
            println!("{}", aegis_data::uses::quarantine(&db, &sha256, &reason)?)
        }
        Command::LineageCheck { db, sha256 } => {
            println!("{}", aegis_data::uses::allow(&db, &sha256)?)
        }
        Command::BrokerQuote {
            request,
            inventory,
            out,
        } => {
            let r = aegis_data::uses::quote(&request, &inventory)?;
            fs::write(out, serde_json::to_vec_pretty(&r)?)?;
            println!("{r}");
            if r["chosen"].is_null() {
                bail!("no eligible worker")
            }
        }
        Command::IncidentDrill {
            workspace,
            binary,
            out,
            scenario,
            python,
            trusted_local,
        } => println!(
            "{}",
            aegis_data::uses::drill(&workspace, &binary, &out, &scenario, &python, trusted_local)?
        ),
        Command::CaseCapture { source, spec, out } => {
            println!("{}", aegis_data::cases::capture(&source, &spec, &out)?)
        }
        Command::CasePropose {
            case,
            generator,
            trusted_local,
        } => println!(
            "{}",
            aegis_data::cases::propose(&case, &generator, trusted_local)?
        ),
        Command::CaseRehearse {
            case,
            patch,
            trusted_local,
        } => {
            let r = aegis_data::cases::rehearse(&case, &patch, trusted_local)?;
            println!("{}", r);
            if r["accepted"] != true {
                bail!("case rejected")
            }
        }
        Command::CaseExport { case, out } => {
            println!("{}", aegis_data::cases::export(&case, &out)?)
        }
        Command::CaseReplay {
            package,
            out,
            exchange_sha256,
            trusted_local,
        } => {
            let r = aegis_data::cases::replay(&package, &exchange_sha256, &out, trusted_local)?;
            println!("{}", r);
            if r["accepted"] != true {
                bail!("replay rejected")
            }
        }
        Command::CasePromote {
            case,
            source,
            report_sha256,
            rollback,
            remote_db,
            remote_job,
            lineage_db,
        } => {
            if remote_db.is_some() != remote_job.is_some() {
                bail!("remote DB and job required together")
            };
            println!(
                "{}",
                aegis_data::cases::promote(
                    &case,
                    &source,
                    &report_sha256,
                    rollback,
                    remote_db.as_deref().zip(remote_job.as_deref()),
                    lineage_db.as_deref()
                )?
            );
        }
        Command::Readiness {
            registry,
            root,
            out,
        } => {
            let report = aegis_data::readiness::report(&root, &registry)?;
            let raw = serde_json::to_string_pretty(&report)?;
            if let Some(path) = out {
                fs::write(path, &raw)?;
            }
            println!("{raw}");
            if report["invalid_evidence_checks"].as_u64().unwrap_or(0) > 0 {
                bail!("readiness evidence missing, changed or mismatched")
            };
        }
        Command::WorkerController {
            db,
            tokens,
            bind,
            ttl,
        } => aegis_data::remote::serve(db, tokens, bind, ttl).await?,
        Command::WorkerRemote {
            controller,
            worker,
            root,
            token_env,
            trusted_local,
        } => {
            println!(
                "{}",
                tokio::task::spawn_blocking(move || aegis_data::remote::run(
                    &controller,
                    &worker,
                    &root,
                    &token_env,
                    trusted_local
                ))
                .await??
            );
        }
        Command::RemoteVerify { db, id } => {
            println!("{}", aegis_data::remote::verify_return(&db, &id)?)
        }
        Command::WorkerEnqueue {
            db,
            plan,
            bundle,
            worker,
        } => {
            println!(
                "{}",
                aegis_data::worker::enqueue_for(&db, &plan, &bundle, worker)?
            )
        }
        Command::WorkerOnce {
            db,
            worker,
            ttl,
            trusted_local,
        } => println!(
            "{}",
            aegis_data::worker::work(&db, &worker, ttl, trusted_local)?
        ),
        Command::WorkerVerify { db, id } => println!("{}", aegis_data::worker::verify(&db, &id)?),
        Command::WorkerStatus { db, id } => println!("{}", Ledger::open(&db)?.inspect(&id)?),
        Command::GraphExtract {
            source,
            source_url,
            out,
        } => fs::write(
            out,
            serde_json::to_vec_pretty(&aegis_data::graph::extract(&source, &source_url)?)?,
        )?,
        Command::OperatorNote {
            db,
            run_id,
            message,
            actor,
        } => {
            aegis_data::operator::open(&db)?.record(&run_id,"operator_note",serde_json::json!({"actor":actor,"text":message,"effect":"recorded context; does not change commands or model weights"}))?;
        }
        Command::OperatorReference {
            db,
            corpus,
            out,
            run_id,
            steps,
            python,
            trusted_local,
            plan_only,
        } => {
            let p = aegis_data::operator::reference_plan(
                std::env::current_dir()?,
                corpus,
                out,
                run_id,
                steps,
                python,
            )?;
            if plan_only {
                println!("{}", serde_json::to_string_pretty(&p)?)
            } else {
                let result = aegis_data::operator::run(&db, p, trusted_local).await?;
                println!("{}", serde_json::to_string_pretty(&result)?);
                if result["status"] != "completed" {
                    bail!("operator reference did not complete")
                }
            }
        }
        Command::GraphDiff { db, before, after } => {
            let g = aegis_data::graph::GraphStore::open(&db)?;
            let a = g.snapshot(Some(before))?;
            let b = g.snapshot(Some(after))?;
            let mut differences = serde_json::Map::new();
            for field in ["nodes", "edges"] {
                let aa: std::collections::BTreeMap<_, _> = a[field]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|n| (n["id"].as_str().unwrap(), n))
                    .collect();
                let bb: std::collections::BTreeMap<_, _> = b[field]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|n| (n["id"].as_str().unwrap(), n))
                    .collect();
                let changed: Vec<_> = bb
                    .iter()
                    .filter(|(id, n)| aa.get(*id) != Some(n))
                    .map(|(id, _)| *id)
                    .collect();
                differences.insert(
                    field.into(),
                    serde_json::json!({"added_or_changed":changed}),
                );
            }
            println!("{}", serde_json::Value::Object(differences));
        }
        Command::RepairPropose { source, out, tool } => {
            println!("{}", aegis_data::repair::propose(&source, &out, &tool)?)
        }
        Command::RepairApply {
            source,
            receipt,
            rollback,
        } => println!(
            "{}",
            aegis_data::repair::apply(&source, &receipt, rollback)?
        ),
        Command::OperatorServe {
            db,
            workspace,
            bind,
            trusted_local,
        } => aegis_data::api::serve(db, workspace, bind, trusted_local).await?,
        Command::OperatorRun {
            db,
            plan,
            trusted_local,
        } => {
            let p: aegis_data::operator::Plan = serde_json::from_slice(&fs::read(plan)?)?;
            let result = aegis_data::operator::run(&db, p, trusted_local).await?;
            println!("{}", serde_json::to_string_pretty(&result)?);
            if result["status"] != "completed" {
                bail!("operator run did not complete")
            }
        }
        Command::OperatorStatus { db, run_id } => {
            println!("{}", aegis_data::operator::status(&db, &run_id)?)
        }
        Command::OperatorControl {
            db,
            run_id,
            action,
            actor,
        } => aegis_data::operator::control(&db, &run_id, &action, &actor)?,
        Command::OperatorEvents { db, run_id, after } => println!(
            "{}",
            serde_json::to_string(&aegis_data::operator::open(&db)?.events(&run_id, after)?)?
        ),
        Command::GraphSnapshot { db, at, out } => fs::write(
            out,
            serde_json::to_vec_pretty(&aegis_data::graph::GraphStore::open(&db)?.snapshot(at)?)?,
        )?,
        Command::GraphImpact { db, id } => {
            println!("{}", aegis_data::graph::GraphStore::open(&db)?.impact(&id)?)
        }
        Command::GraphVerify { db } => println!(
            "events_verified={}",
            aegis_data::graph::GraphStore::open(&db)?.verify()?
        ),
        Command::GraphImportCatalog { db, catalog } => {
            let c: serde_json::Value = serde_json::from_slice(&fs::read(catalog)?)?;
            let mut g = aegis_data::graph::GraphStore::open(&db)?;
            for d in c["definitions"]
                .as_array()
                .ok_or_else(|| anyhow::anyhow!("invalid catalog"))?
            {
                g.node(
                    "catalog",
                    d["id"]
                        .as_str()
                        .ok_or_else(|| anyhow::anyhow!("missing id"))?,
                    "graph-definition",
                    d.clone(),
                )?;
            }
        }
        Command::Renew {
            db,
            id,
            worker,
            epoch,
            ttl,
        } => println!(
            "expires={}",
            Ledger::open(&db)?.renew(
                &aegis_data::ledger::Lease {
                    id,
                    worker,
                    epoch,
                    expires: 0
                },
                now()?,
                ttl
            )?
        ),
        Command::Cancel { db, id } => println!("cancelled={}", Ledger::open(&db)?.cancel(&id)?),
        Command::ValidateDataset { root } => {
            let s = ingest::validate_dataset(&root)?;
            println!("validated {} records", s.train + s.validation + s.test)
        }
        Command::Compare {
            baseline,
            candidate,
            out,
        } => {
            let a: aegis_data::experiment::Experiment =
                serde_json::from_slice(&fs::read(baseline)?)?;
            let b: aegis_data::experiment::Experiment =
                serde_json::from_slice(&fs::read(candidate)?)?;
            fs::write(
                out,
                serde_json::to_vec_pretty(&aegis_data::experiment::compare(&a, &b)?)?,
            )?
        }
        Command::CodeScan {
            root,
            registry,
            out,
        } => {
            let (i, _) = aegis_data::code::scan(&root, &registry)?;
            fs::write(out, serde_json::to_vec_pretty(&i)?)?;
            println!("{} source files", i.files.len());
        }
        Command::CodeCheck {
            root,
            registry,
            out,
            timeout,
            trusted_local,
            languages,
        } => {
            if !trusted_local {
                bail!("local tool execution requires --trusted-local; use disposable isolated workers for untrusted projects")
            };
            let report = aegis_data::code::check(&root, &registry, timeout, &languages)?;
            fs::write(out, serde_json::to_vec_pretty(&report)?)?;
            if !report.complete {
                bail!("checks incomplete or failing; see report")
            }
        }
        Command::Tokenize {
            input,
            out,
            tokens,
            max_rows,
        } => println!(
            "rows={}",
            aegis_data::tokenize::export(&input, &out, tokens, max_rows)?
        ),
        Command::Prepare {
            input,
            out,
            prefetch,
        } => println!(
            "{}",
            serde_json::to_string_pretty(&ingest::prepare(input, out, prefetch).await?)?
        ),
        Command::Inspect { input } => println!("records={}", ingest::inspect(&input)?),
        Command::Classify { input } => {
            let x = serde_json::from_slice(&fs::read(input)?)?;
            println!(
                "{}",
                serde_json::to_string_pretty(&metrics::classification(&x)?)?
            )
        }
        Command::Regression { input } => {
            let x = serde_json::from_slice(&fs::read(input)?)?;
            println!(
                "{}",
                serde_json::to_string_pretty(&metrics::regression(&x)?)?
            )
        }
        Command::Gate { input, out } => {
            let g: Vec<metrics::Gate> = serde_json::from_slice(&fs::read(&input)?)?;
            let verdict = metrics::gates(&g)?;
            println!("{}", serde_json::to_string_pretty(&verdict)?);
            if let Some(out) = out {
                fs::write(
                    out,
                    serde_json::to_vec_pretty(
                        &serde_json::json!({"schema_version":1,"accepted":verdict.iter().all(|v|v.pass),"verdicts":verdict,"gate_config_sha256":ingest::hash_file(&input)?}),
                    )?,
                )?;
            }
            if verdict.iter().any(|v| !v.pass) {
                bail!("release gate failed")
            }
        }
        Command::Plan { input } => {
            let c: Cluster = serde_yaml::from_slice(&fs::read(input)?)?;
            let mut names = std::collections::HashSet::new();
            if c.nodes.is_empty()
                || !(1..=64).contains(&c.prefetch)
                || c.max_tokens == 0
                || c.nodes
                    .iter()
                    .any(|n| n.id.is_empty() || !names.insert(&n.id))
            {
                bail!("invalid cluster config")
            }
            let assignments: Vec<_> = c
                .shards
                .iter()
                .enumerate()
                .map(|(i, s)| serde_json::json!({"shard":s,"node":c.nodes[i%c.nodes.len()].id}))
                .collect();
            println!(
                "{}",
                serde_json::to_string_pretty(
                    &serde_json::json!({"status":"plan_only","prefetch":c.prefetch,"max_tokens":c.max_tokens,"assignments":assignments})
                )?
            );
        }
        Command::Enqueue { db, id, payload } => {
            println!("inserted={}", Ledger::open(&db)?.enqueue(&id, &payload)?)
        }
        Command::Claim { db, worker, ttl } => println!(
            "{}",
            serde_json::to_string(&Ledger::open(&db)?.claim(&worker, now()?, ttl)?)?
        ),
        Command::Complete {
            db,
            id,
            worker,
            epoch,
            result,
        } => {
            Ledger::open(&db)?.complete(
                &aegis_data::ledger::Lease {
                    id,
                    worker,
                    epoch,
                    expires: 0,
                },
                now()?,
                &result,
            )?;
            println!("completed")
        }
    }
    Ok(())
}

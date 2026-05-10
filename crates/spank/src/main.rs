//! `spank` — entry point.
//!
//! Subcommands:
//! - `serve` — boot the API + HEC + (optional) TCP receiver.
//! - `ship` — read a file and ship its lines via TCP.
//! - `show-config` — print the effective merged config.

mod cli;

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Context;
use clap::{Parser, Subcommand};
use spank_cfg::{load, render_toml, FileMode};
use spank_core::lifecycle::Lifecycle;
use spank_obs::{init_tracing, install_prometheus, lifecycle_event, MetricsHandle};

#[derive(Parser, Debug)]
#[command(name = "spank", version, about = "Spank — Splunk-compatible log search")]
struct Cli {
    /// Path to a TOML config file.
    #[arg(long, env = "SP_CONFIG", global = true)]
    config: Option<PathBuf>,

    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand, Debug)]
enum Cmd {
    /// Run the API + HEC + optional TCP receiver.
    Serve,
    /// Read a file and ship its lines via TCP.
    Ship {
        /// Source file path.
        #[arg(long)]
        from: PathBuf,
        /// Destination host:port.
        #[arg(long)]
        to: String,
    },
    /// Print effective merged config and exit.
    ShowConfig,
    /// Print baseline workload (CPU + a small SQLite insert) and exit.
    Bench,
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let cfg = load(cli.config.as_deref()).context("load config")?;
    init_tracing(&cfg.tracing).context("init tracing")?;
    let metrics = Arc::new(install_prometheus().map_err(anyhow::Error::msg)?);

    // Install panic hook after the Prometheus recorder is registered so that
    // the counter increment has a live recorder to write to. Panics between
    // process start and this line are handled by the default hook only.
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        metrics::counter!(spank_obs::metrics::names::PANICS_TOTAL).increment(1);
        default_hook(info);
    }));

    let runtime = build_runtime(&cfg.runtime)?;

    runtime.block_on(async move {
        match cli.cmd {
            Cmd::Serve => serve(cfg, metrics).await,
            Cmd::Ship { from, to } => ship(from, to).await,
            Cmd::ShowConfig => {
                println!("{}", render_toml(&cfg).map_err(anyhow::Error::msg)?);
                Ok(())
            }
            Cmd::Bench => bench(cfg).await,
        }
    })
}

fn build_runtime(rcfg: &spank_cfg::RuntimeConfig) -> anyhow::Result<tokio::runtime::Runtime> {
    let mut b = tokio::runtime::Builder::new_multi_thread();
    b.enable_all();
    if let Some(n) = rcfg.worker_threads {
        b.worker_threads(n);
    }
    b.thread_name("spank-worker");
    Ok(b.build()?)
}

async fn serve(
    cfg: spank_cfg::SpankConfig,
    metrics: Arc<MetricsHandle>,
) -> anyhow::Result<()> {
    let lifecycle = Lifecycle::root();

    // Collect known index names from token allowed_indexes for the indexes endpoint.
    let known_indexes = {
        let mut names: Vec<String> = cfg
            .hec
            .tokens
            .iter()
            .flat_map(|t| t.allowed_indexes.iter().cloned())
            .collect();
        names.sort();
        names.dedup();
        names
    };

    // API state.
    let api_state = spank_api::ApiState::new(metrics.clone(), "shank", known_indexes);

    // HEC wiring (channel, sender, consumer, routes).
    let drain = spank_core::Drain::new();
    let (hec_tx, hec_rx) =
        tokio::sync::mpsc::channel::<spank_hec::receiver::QueueItem>(cfg.hec.queue_depth);
    let token_store = spank_hec::TokenStore::from_config(&cfg.hec.tokens);
    let auth: Arc<dyn spank_hec::Authenticator> =
        Arc::new(spank_hec::HecTokenAuthenticator::new(token_store));
    let file_sender = Arc::new(spank_hec::FileSender::new(cfg.hec.output_dir.clone())?);
    let consumer = spank_hec::receiver::spawn_consumer(
        hec_rx,
        file_sender,
        drain.clone(),
        lifecycle.child("hec_consumer"),
    );

    let hec_state = Arc::new(spank_hec::receiver::HecState {
        auth,
        queue: hec_tx,
        max_content_length: cfg.hec.max_content_length,
        phase: api_state.phase.clone(),
        drain: drain.clone(),
    });

    // TCP receiver wiring (optional).
    let tcp_handle = if let Some(bind) = &cfg.tcp.bind {
        let addr: std::net::SocketAddr = bind.parse().context("parse tcp.bind")?;
        let (tcp_tx, tcp_rx) = tokio::sync::mpsc::channel::<spank_tcp::ConnEvent>(1024);
        let tcp_lc = lifecycle.child("tcp_listener");
        let max_line_bytes = cfg.tcp.max_line_bytes;
        let output_dir = cfg.tcp.output_dir.clone();
        let consumer = spawn_tcp_to_files(tcp_rx, output_dir);
        let listen = tokio::spawn(async move {
            spank_tcp::serve(addr, max_line_bytes, tcp_tx, tcp_lc).await
        });
        Some((listen, consumer))
    } else {
        None
    };

    // Mark phase SERVING once subsystems are constructed. The transition
    // from STARTED to SERVING is the one permitted path; enforce it through
    // can_transition_to so that future callers cannot bypass the state machine.
    let current = api_state.current_phase();
    let next = spank_core::HecPhase::SERVING;
    if current.can_transition_to(next) {
        api_state.set_phase(next);
    } else {
        return Err(anyhow::anyhow!(
            "illegal phase transition {:?} -> {:?}; aborting startup",
            current,
            next
        ));
    }

    // Build router: API + HEC.
    let router = spank_api::router::build(api_state.clone())
        .merge(spank_hec::routes(hec_state));

    let api_addr: std::net::SocketAddr = cfg.api.bind.parse().context("parse api.bind")?;

    // Signal handlers.
    let lc_for_signals = lifecycle.clone();
    tokio::spawn(async move {
        match tokio::signal::ctrl_c().await {
            Ok(()) => {
                lifecycle_event!(component = "process", kind = "signal", signal = "ctrl_c");
                lc_for_signals.shutdown();
            }
            Err(e) => {
                tracing::warn!(error = %e, "ctrl_c handler failed");
            }
        }
    });

    let api_lc = lifecycle.child("api");
    let api_join = tokio::spawn(async move {
        spank_api::serve(router, api_addr, api_lc).await
    });

    // Wait for shutdown.
    let _ = api_join.await?;
    lifecycle.shutdown();

    // Drain the HEC consumer. Joining the task handle provides the same
    // ordering guarantee as drain.wait() for the current single-consumer
    // topology: the task exits only after processing all queued items.
    // Per-tag drain.wait() is required when the HEC ACK endpoint lands
    // (Plan.md OBS-DRAIN1): at that point a tag registry must be maintained
    // and each active tag waited before the response is sent.
    let shutdown_budget = std::time::Duration::from_secs(cfg.runtime.shutdown_seconds);
    let _ = tokio::time::timeout(shutdown_budget, consumer).await;
    if let Some((listen, consume)) = tcp_handle {
        let _ = tokio::time::timeout(shutdown_budget, listen).await;
        let _ = tokio::time::timeout(shutdown_budget, consume).await;
    }

    lifecycle_event!(component = "process", kind = "exit");
    Ok(())
}

fn spawn_tcp_to_files(
    mut rx: tokio::sync::mpsc::Receiver<spank_tcp::ConnEvent>,
    output_dir: PathBuf,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let _ = std::fs::create_dir_all(&output_dir);
        use std::collections::HashMap;
        use std::fs::OpenOptions;
        use std::io::Write;
        let mut writers: HashMap<u64, std::io::BufWriter<std::fs::File>> = HashMap::new();
        while let Some(ev) = rx.recv().await {
            match ev {
                spank_tcp::ConnEvent::Opened { handle } => {
                    let path = output_dir.join(format!(
                        "{}-{}.log",
                        handle.peer.ip().to_string().replace(':', "_"),
                        handle.conn_id
                    ));
                    if let Ok(f) = OpenOptions::new()
                        .create(true)
                        .append(true)
                        .open(&path)
                    {
                        writers.insert(handle.conn_id, std::io::BufWriter::new(f));
                    }
                }
                spank_tcp::ConnEvent::Line { handle, line } => {
                    if let Some(w) = writers.get_mut(&handle.conn_id) {
                        let _ = w.write_all(line.as_bytes());
                        let _ = w.write_all(b"\n");
                    }
                }
                spank_tcp::ConnEvent::Closed { handle, .. } => {
                    if let Some(mut w) = writers.remove(&handle.conn_id) {
                        let _ = w.flush();
                    }
                }
            }
        }
    })
}

async fn ship(from: PathBuf, to: String) -> anyhow::Result<()> {
    let lifecycle = Lifecycle::root();
    let (line_tx, mut line_rx) = tokio::sync::mpsc::channel::<spank_files::monitor::FileOutput>(1024);
    let (ship_tx, ship_rx) = tokio::sync::mpsc::channel::<String>(1024);

    let monitor = spank_files::FileMonitor::new(from.clone(), FileMode::OneShot, 1024);
    let mon_lc = lifecycle.child("file_monitor");
    let mon = tokio::spawn(async move { monitor.run(line_tx, mon_lc).await });

    let sender = spank_shipper::TcpSender::new(to);
    let send_lc = lifecycle.child("tcp_sender");
    let send = tokio::spawn(async move { sender.run(ship_rx, send_lc).await });

    // Bridge file lines to ship channel until the file emits Done.
    let bridge = tokio::spawn(async move {
        while let Some(out) = line_rx.recv().await {
            match out {
                spank_files::monitor::FileOutput::Line(fl) => {
                    if ship_tx.send(fl.line).await.is_err() {
                        break;
                    }
                }
                spank_files::monitor::FileOutput::Done(_s) => break,
            }
        }
    });

    let _ = mon.await?;
    let _ = bridge.await;
    lifecycle.shutdown();
    let _ = tokio::time::timeout(std::time::Duration::from_secs(2), send).await;
    Ok(())
}

async fn bench(cfg: spank_cfg::SpankConfig) -> anyhow::Result<()> {
    use std::time::Instant;
    use spank_core::Record;
    use spank_store::{SqliteBackend, traits::PartitionManager};

    let dir = tempfile::tempdir()?;
    let backend = SqliteBackend::open(dir.path())?;
    let mut w = backend.create_hot("bench")?;
    let n: usize = 100_000;
    let rows: Vec<Record> = (0..n)
        .map(|i| {
            Record::builder(format!("line {i}"))
                .time_event_ns(i as i64)
                .time_index_ns(i as i64)
                .source("bench")
                .sourcetype("test")
                .build()
        })
        .collect();
    let t0 = Instant::now();
    w.append(&rows)?;
    w.commit()?;
    let dt = t0.elapsed();
    let ips = n as f64 / dt.as_secs_f64();
    println!(
        "sqlite bulk_insert n={} elapsed_ms={} inserts_per_sec={:.0}",
        n,
        dt.as_millis(),
        ips
    );
    let _ = cfg;
    Ok(())
}

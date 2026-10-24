// SPDX-License-Identifier: Apache-2.0

//! The `ferrogate` binary.

use std::io::Write;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Duration;

use anyhow::Context;
use clap::{Args, Parser, Subcommand, ValueEnum};
use ferrogate::serve;
use ferrogate_crd::ClusterGatePolicy;
use kube::CustomResourceExt;

#[derive(Parser)]
#[command(name = "ferrogate", version, about)]
struct Cli {
    /// Log output format.
    #[arg(long, value_enum, default_value_t = LogFormat::Text, env = "FERROGATE_LOG_FORMAT", global = true)]
    log_format: LogFormat,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Run the controller: policy status, events and background scans.
    Operator(OperatorArgs),
    /// Run the admission webhook (HTTPS).
    Webhook(WebhookArgs),
    /// Print the custom resource definitions as YAML.
    Crd,
    /// Check local YAML files against policies, without a cluster. Exits with 1 if any check fails.
    Check {
        /// Policy files or directories (`ClusterGatePolicy` documents).
        #[arg(short, long = "policy", required = true)]
        policies: Vec<PathBuf>,
        /// Resource files or directories to check.
        #[arg(required = true)]
        resources: Vec<PathBuf>,
        /// Report format.
        #[arg(short, long, value_enum, default_value_t = Output::Text)]
        output: Output,
    },
}

#[derive(Args)]
struct OperatorArgs {
    /// Address of the operations server (/healthz, /readyz, /metrics).
    #[arg(long, default_value = "0.0.0.0:8080", env = "FERROGATE_METRICS_ADDR")]
    metrics_addr: SocketAddr,
    /// Disable background scans of existing resources.
    #[arg(long, env = "FERROGATE_NO_SCAN")]
    no_scan: bool,
    /// Seconds between background scans of each policy.
    #[arg(long, default_value_t = 300, env = "FERROGATE_SCAN_INTERVAL", value_parser = clap::value_parser!(u64).range(30..))]
    scan_interval: u64,
    /// Maximum resources evaluated per policy and scan.
    #[arg(long, default_value_t = 20_000, env = "FERROGATE_MAX_SCAN_OBJECTS")]
    max_scan_objects: usize,
    /// Elect a leader through a Lease so several replicas can run. Needs `POD_NAME`.
    #[arg(long, env = "FERROGATE_LEADER_ELECT")]
    leader_elect: bool,
    /// Namespace of the leader election Lease.
    #[arg(long, env = "POD_NAMESPACE", default_value = "ferrogate-system")]
    lease_namespace: String,
}

#[derive(Args)]
struct WebhookArgs {
    /// Address of the HTTPS admission server.
    #[arg(long, default_value = "0.0.0.0:8443", env = "FERROGATE_WEBHOOK_ADDR")]
    addr: SocketAddr,
    /// Serving certificate (PEM). Reloaded when the file changes.
    #[arg(long, default_value = "/tls/tls.crt", env = "FERROGATE_TLS_CERT")]
    tls_cert: PathBuf,
    /// Serving private key (PEM). Reloaded when the file changes.
    #[arg(long, default_value = "/tls/tls.key", env = "FERROGATE_TLS_KEY")]
    tls_key: PathBuf,
    /// Address of the operations server (/healthz, /readyz, /metrics).
    #[arg(long, default_value = "0.0.0.0:8080", env = "FERROGATE_METRICS_ADDR")]
    metrics_addr: SocketAddr,
    /// Largest accepted `AdmissionReview` body, in bytes.
    #[arg(long, default_value_t = ferrogate_webhook::DEFAULT_MAX_BODY_BYTES, env = "FERROGATE_MAX_BODY_BYTES")]
    max_body_bytes: usize,
}

#[derive(Clone, Copy, ValueEnum)]
enum Output {
    Text,
    Json,
}

#[derive(Clone, Copy, ValueEnum)]
enum LogFormat {
    Text,
    Json,
}

#[tokio::main]
async fn main() -> anyhow::Result<ExitCode> {
    let cli = Cli::parse();

    match cli.command {
        Command::Crd => print_crds().map(|()| ExitCode::SUCCESS),
        Command::Check {
            policies,
            resources,
            output,
        } => check(&policies, &resources, output),
        Command::Operator(args) => {
            init_logging(cli.log_format);
            tracing::info!(
                version = env!("CARGO_PKG_VERSION"),
                "starting ferrogate operator"
            );
            let client = connect().await?;
            let config = ferrogate_operator::Config {
                scan: !args.no_scan,
                scan_interval: Duration::from_secs(args.scan_interval),
                max_scan_objects: args.max_scan_objects,
                instance: std::env::var("POD_NAME").ok(),
                ..ferrogate_operator::Config::default()
            };
            let lease = if args.leader_elect {
                let identity = std::env::var("POD_NAME")
                    .context("--leader-elect needs the POD_NAME environment variable")?;
                Some(ferrogate_operator::leader::LeaseConfig::new(
                    args.lease_namespace,
                    identity,
                ))
            } else {
                None
            };
            serve::operator(client, config, args.metrics_addr, lease).await?;
            Ok(ExitCode::SUCCESS)
        }
        Command::Webhook(args) => {
            init_logging(cli.log_format);
            tracing::info!(
                version = env!("CARGO_PKG_VERSION"),
                "starting ferrogate webhook"
            );
            let client = connect().await?;
            let limits = ferrogate_webhook::Limits {
                max_body_bytes: args.max_body_bytes,
                ..ferrogate_webhook::Limits::default()
            };
            let tls = ferrogate_webhook::TlsFiles {
                cert: args.tls_cert,
                key: args.tls_key,
            };
            serve::webhook(client, args.addr, args.metrics_addr, tls, limits).await?;
            Ok(ExitCode::SUCCESS)
        }
    }
}

async fn connect() -> anyhow::Result<kube::Client> {
    kube::Client::try_default()
        .await
        .context("could not connect to Kubernetes (check kubeconfig or service account)")
}

fn print_crds() -> anyhow::Result<()> {
    let yaml = serde_saphyr::to_string(&ClusterGatePolicy::crd()).context("serialize CRD")?;
    let mut out = std::io::stdout().lock();
    writeln!(out, "# Generated by `ferrogate crd`. Do not edit.")?;
    writeln!(out, "---")?;
    write!(out, "{yaml}")?;
    Ok(())
}

fn check(policies: &[PathBuf], resources: &[PathBuf], output: Output) -> anyhow::Result<ExitCode> {
    use ferrogate::check;
    let policy_docs = check::load(policies)?;
    let compiled = check::policies(&policy_docs)?;
    if compiled.is_empty() {
        anyhow::bail!("no ClusterGatePolicy found in the --policy paths");
    }
    let documents = check::load(resources)?;
    let results = check::run(&compiled, &documents);
    let report = match output {
        Output::Text => check::text_report(&results, compiled.len()),
        Output::Json => format!("{:#}\n", check::json_report(&results)),
    };
    std::io::stdout().lock().write_all(report.as_bytes())?;
    Ok(if results.iter().any(check::Checked::failed) {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    })
}

fn init_logging(format: LogFormat) {
    let filter = tracing_subscriber::EnvFilter::try_from_env("FERROGATE_LOG")
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));
    let builder = tracing_subscriber::fmt().with_env_filter(filter);
    match format {
        LogFormat::Text => builder.init(),
        LogFormat::Json => builder.json().init(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn cli_definition_is_valid() {
        Cli::command().debug_assert();
    }

    /// Building an HTTPS client panics at runtime if no TLS crypto provider is compiled in.
    #[tokio::test]
    async fn https_kube_client_can_be_built() {
        let config = kube::Config::new("https://127.0.0.1:6443".parse().unwrap());
        kube::Client::try_from(config).unwrap();
    }
}

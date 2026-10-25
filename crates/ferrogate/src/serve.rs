// SPDX-License-Identifier: Apache-2.0

//! Wiring for the long-running subcommands: servers, caches and shutdown.

use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::sync::Arc;

use ferrogate_core::Policy;
use ferrogate_operator::cache::Caches;
use ferrogate_operator::leader::{self, LeaseConfig};
use ferrogate_webhook::{Limits, PolicySource, TlsFiles, WebhookMetrics};
use prometheus_client::registry::Registry;
use tokio::sync::watch;

/// The webhook's view of the watch caches.
struct CacheSource(Arc<Caches>);

impl PolicySource for CacheSource {
    fn policies(&self) -> Arc<Vec<Policy>> {
        self.0.policies()
    }

    fn namespace_labels(&self, namespace: &str) -> Option<BTreeMap<String, String>> {
        self.0.namespace_labels(namespace)
    }

    fn is_ready(&self) -> bool {
        self.0.is_ready()
    }
}

/// A shutdown signal that many tasks can wait on. Fires on SIGTERM or Ctrl-C.
#[derive(Clone)]
pub struct Shutdown(watch::Receiver<bool>);

impl Shutdown {
    pub fn on_signals() -> Self {
        let (tx, rx) = watch::channel(false);
        tokio::spawn(async move {
            wait_for_signal().await;
            tracing::info!("shutdown signal received");
            let _ = tx.send(true);
        });
        Self(rx)
    }

    pub async fn wait(mut self) {
        let _ = self.0.wait_for(|stop| *stop).await;
    }
}

async fn wait_for_signal() {
    let ctrl_c = async {
        if let Err(error) = tokio::signal::ctrl_c().await {
            tracing::warn!(%error, "cannot listen for Ctrl-C");
            std::future::pending::<()>().await;
        }
    };
    #[cfg(unix)]
    let terminate = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut signal) => {
                signal.recv().await;
            }
            Err(error) => {
                tracing::warn!(%error, "cannot listen for SIGTERM");
                std::future::pending::<()>().await;
            }
        }
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();
    tokio::select! {
        () = ctrl_c => {}
        () = terminate => {}
    }
}

/// Runs the controller and its operations server. With `lease`, waits to be elected first and
/// stops with an error if leadership is lost, so a deposed replica never keeps reconciling.
pub async fn operator(
    client: kube::Client,
    config: ferrogate_operator::Config,
    metrics_addr: SocketAddr,
    lease: Option<LeaseConfig>,
) -> anyhow::Result<()> {
    let shutdown = Shutdown::on_signals();
    let mut registry = Registry::default();
    let metrics = ferrogate_operator::OperatorMetrics::register(&mut registry);
    // Standby replicas are healthy and ready: readiness says "can take over", not "is leader".
    let ops = ferrogate_webhook::ops_router(Arc::new(registry), Arc::new(|| true));
    let ops_server = tokio::spawn(ferrogate_webhook::serve_plain(
        metrics_addr,
        ops,
        shutdown.clone().wait(),
    ));

    let result = match lease {
        None => {
            ferrogate_operator::run(client, config, metrics).await;
            Ok(())
        }
        Some(lease) => run_elected(client, config, metrics, lease, shutdown.clone()).await,
    };
    ops_server.await??;
    result
}

async fn run_elected(
    client: kube::Client,
    config: ferrogate_operator::Config,
    metrics: ferrogate_operator::OperatorMetrics,
    lease: LeaseConfig,
    shutdown: Shutdown,
) -> anyhow::Result<()> {
    let (tx, mut leader) = watch::channel(false);
    let mut election = tokio::spawn(leader::run(client.clone(), lease.clone(), tx));
    tracing::info!(lease = %lease.name, identity = %lease.identity, "waiting for leadership");
    tokio::select! {
        elected = leader.wait_for(|is_leader| *is_leader) => {
            if elected.is_err() {
                anyhow::bail!("leader election stopped");
            }
        }
        () = shutdown.clone().wait() => {
            election.abort();
            return Ok(());
        }
    }

    let result = tokio::select! {
        () = ferrogate_operator::run(client.clone(), config, metrics) => Ok(()),
        lost = &mut election => match lost {
            Ok(error) => Err(anyhow::Error::new(error).context("stopping so that only the new leader reconciles")),
            Err(join) => Err(anyhow::anyhow!("leader election task failed: {join}")),
        },
    };
    election.abort();
    leader::release(client, &lease).await;
    result
}

/// Runs the admission webhook, its watch caches and its operations server.
pub async fn webhook(
    client: kube::Client,
    addr: SocketAddr,
    metrics_addr: SocketAddr,
    tls: TlsFiles,
    limits: Limits,
) -> anyhow::Result<()> {
    let shutdown = Shutdown::on_signals();
    let (caches, run_caches) = Caches::start(&client);
    tokio::spawn(run_caches);

    let mut registry = Registry::default();
    let metrics = WebhookMetrics::register(&mut registry);
    let ready_caches = caches.clone();
    let ops = ferrogate_webhook::ops_router(
        Arc::new(registry),
        Arc::new(move || ready_caches.is_ready()),
    );
    let admission =
        ferrogate_webhook::admission_router(Arc::new(CacheSource(caches)), metrics, limits);

    let ops_server = tokio::spawn(ferrogate_webhook::serve_plain(
        metrics_addr,
        ops,
        shutdown.clone().wait(),
    ));
    ferrogate_webhook::serve_tls(addr, admission, tls, shutdown.wait()).await?;
    ops_server.await??;
    tracing::info!("webhook stopped");
    Ok(())
}

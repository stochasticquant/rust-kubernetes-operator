// SPDX-License-Identifier: Apache-2.0

//! Wiring for the long-running subcommands: servers, caches and shutdown.

use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::sync::Arc;

use ferrogate_core::Policy;
use ferrogate_operator::cache::Caches;
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

/// Runs the controller and its operations server.
pub async fn operator(
    client: kube::Client,
    config: ferrogate_operator::Config,
    metrics_addr: SocketAddr,
) -> anyhow::Result<()> {
    let shutdown = Shutdown::on_signals();
    let mut registry = Registry::default();
    let metrics = ferrogate_operator::OperatorMetrics::register(&mut registry);
    let ops = ferrogate_webhook::ops_router(Arc::new(registry), Arc::new(|| true));
    let ops_server = tokio::spawn(ferrogate_webhook::serve_plain(
        metrics_addr,
        ops,
        shutdown.clone().wait(),
    ));

    ferrogate_operator::run(client, config, metrics).await;
    ops_server.await??;
    Ok(())
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

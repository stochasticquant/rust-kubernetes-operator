// SPDX-License-Identifier: Apache-2.0

//! Watch caches for policies and namespace labels, used by the admission webhook.
//!
//! Policies are compiled once per change, not per request. Namespaces are watched as metadata
//! only, so their contents never reach this process.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, PoisonError, RwLock};

use ferrogate_core::Policy;
use ferrogate_crd::ClusterGatePolicy;
use futures::future::BoxFuture;
use futures::{FutureExt, StreamExt};
use k8s_openapi::api::core::v1::Namespace;
use kube::core::PartialObjectMeta;
use kube::runtime::reflector::{self, ObjectRef, Store};
use kube::runtime::{WatchStreamExt, watcher};
use kube::{Api, Client, ResourceExt};

/// Compiled policies and namespace labels, kept up to date by watches.
pub struct Caches {
    policies: RwLock<Arc<Vec<Policy>>>,
    namespaces: Store<PartialObjectMeta<Namespace>>,
    policies_ready: AtomicBool,
    namespaces_ready: AtomicBool,
}

impl Caches {
    /// Creates the caches and returns the future that keeps them updated. The future runs until
    /// the process stops; spawn it.
    pub fn start(client: &Client) -> (Arc<Self>, BoxFuture<'static, ()>) {
        let (ns_reader, ns_writer) = reflector::store();
        let (policy_reader, policy_writer) = reflector::store::<ClusterGatePolicy>();
        let caches = Arc::new(Self {
            policies: RwLock::new(Arc::new(Vec::new())),
            namespaces: ns_reader,
            policies_ready: AtomicBool::new(false),
            namespaces_ready: AtomicBool::new(false),
        });

        let namespaces = {
            let caches = caches.clone();
            // Metadata-only requests: namespace specs and statuses are never fetched.
            let api: Api<PartialObjectMeta<Namespace>> = Api::all(client.clone());
            watcher(api, watcher::Config::default())
                .default_backoff()
                .reflect(ns_writer)
                .for_each(move |event| {
                    match event {
                        Ok(watcher::Event::InitDone) => {
                            caches.namespaces_ready.store(true, Ordering::Release);
                            tracing::info!("namespace cache synced");
                        }
                        Ok(_) => {}
                        Err(error) => tracing::warn!(%error, "namespace watch error"),
                    }
                    std::future::ready(())
                })
        };

        let policies = {
            let caches = caches.clone();
            let api: Api<ClusterGatePolicy> = Api::all(client.clone());
            watcher(api, watcher::Config::default())
                .default_backoff()
                .reflect(policy_writer)
                .for_each(move |event| {
                    match event {
                        Ok(watcher::Event::InitDone) => {
                            caches.rebuild(&policy_reader);
                            caches.policies_ready.store(true, Ordering::Release);
                            tracing::info!("policy cache synced");
                        }
                        Ok(watcher::Event::Apply(_) | watcher::Event::Delete(_)) => {
                            caches.rebuild(&policy_reader);
                        }
                        Ok(_) => {}
                        Err(error) => tracing::warn!(%error, "policy watch error"),
                    }
                    std::future::ready(())
                })
        };

        let run = async move {
            futures::join!(namespaces, policies);
        }
        .boxed();
        (caches, run)
    }

    fn rebuild(&self, store: &Store<ClusterGatePolicy>) {
        let mut compiled: Vec<Policy> = store
            .state()
            .iter()
            .filter_map(|object| match object.compile() {
                Ok(policy) => Some(policy),
                Err(error) => {
                    // The controller reports this on the policy's status.
                    tracing::debug!(policy = %object.name_any(), %error, "skipping invalid policy");
                    None
                }
            })
            .collect();
        compiled.sort_by(|a, b| a.name.cmp(&b.name));
        tracing::debug!(count = compiled.len(), "policy cache updated");
        *self
            .policies
            .write()
            .unwrap_or_else(PoisonError::into_inner) = Arc::new(compiled);
    }

    /// The current valid policies, sorted by name.
    pub fn policies(&self) -> Arc<Vec<Policy>> {
        self.policies
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// Labels of `namespace`, if it is in the cache.
    pub fn namespace_labels(&self, namespace: &str) -> Option<BTreeMap<String, String>> {
        self.namespaces
            .get(&ObjectRef::new(namespace))
            .map(|ns| ns.labels().clone())
    }

    /// True once both caches have completed their initial list.
    pub fn is_ready(&self) -> bool {
        self.policies_ready.load(Ordering::Acquire) && self.namespaces_ready.load(Ordering::Acquire)
    }
}

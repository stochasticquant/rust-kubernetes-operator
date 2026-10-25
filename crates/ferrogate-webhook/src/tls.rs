// SPDX-License-Identifier: Apache-2.0

//! TLS for the webhook: rustls with the `ring` provider, TLS 1.2 and 1.3 only, certificate files
//! reloaded when they change (cert-manager rotates them in place).

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use axum_server::tls_rustls::RustlsConfig;
use rustls::ServerConfig;
use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, PrivateKeyDer};

use crate::Error;

/// How often the certificate files are checked for changes.
const RELOAD_INTERVAL: Duration = Duration::from_secs(30);

/// Paths of the serving certificate (PEM chain) and private key (PEM).
#[derive(Clone, Debug)]
pub struct TlsFiles {
    pub cert: PathBuf,
    pub key: PathBuf,
}

impl TlsFiles {
    fn read(&self) -> Result<(Vec<u8>, Vec<u8>), Error> {
        let read = |path: &Path| {
            std::fs::read(path).map_err(|source| Error::Read {
                path: path.to_path_buf(),
                source,
            })
        };
        Ok((read(&self.cert)?, read(&self.key)?))
    }
}

/// Builds a rustls server config from PEM bytes.
pub fn server_config(cert_pem: &[u8], key_pem: &[u8]) -> Result<ServerConfig, Error> {
    let certs: Vec<CertificateDer<'static>> = CertificateDer::pem_slice_iter(cert_pem)
        .collect::<Result<_, _>>()
        .map_err(|e| Error::Pem(format!("certificate: {e}")))?;
    if certs.is_empty() {
        return Err(Error::Pem(
            "certificate file contains no certificate".into(),
        ));
    }
    let key =
        PrivateKeyDer::from_pem_slice(key_pem).map_err(|e| Error::Pem(format!("key: {e}")))?;

    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let mut config = ServerConfig::builder_with_provider(provider)
        .with_protocol_versions(&[&rustls::version::TLS13, &rustls::version::TLS12])?
        .with_no_client_auth()
        .with_single_cert(certs, key)?;
    config.alpn_protocols = vec![b"h2".to_vec(), b"http/1.1".to_vec()];
    Ok(config)
}

/// Loads the certificate and starts a background task that reloads it when the files change.
pub fn load_and_watch(files: TlsFiles) -> Result<RustlsConfig, Error> {
    let (cert, key) = files.read()?;
    let config = RustlsConfig::from_config(Arc::new(server_config(&cert, &key)?));
    let watched = config.clone();
    tokio::spawn(async move {
        let mut current = (cert, key);
        let mut ticker = tokio::time::interval(RELOAD_INTERVAL);
        ticker.tick().await;
        loop {
            ticker.tick().await;
            let next = match files.read() {
                Ok(next) if next != current => next,
                Ok(_) => continue,
                Err(error) => {
                    tracing::warn!(%error, "could not read TLS files; keeping current certificate");
                    continue;
                }
            };
            match server_config(&next.0, &next.1) {
                Ok(config) => {
                    watched.reload_from_config(Arc::new(config));
                    current = next;
                    tracing::info!("reloaded TLS certificate");
                }
                Err(error) => {
                    tracing::warn!(%error, "new TLS files are invalid; keeping current certificate");
                }
            }
        }
    });
    Ok(config)
}

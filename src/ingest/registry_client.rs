//! The ingester's only data source: the registry's HTTP surface (`/health`,
//! `current`, `blob/{sha256}`). Blob bodies are streamed to a blocking parser
//! through a bounded channel and verified (size and SHA-256) on the way.

use std::io::Read;
use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use axum::body::Bytes;
use sha2::Digest as _;

use crate::api::internal::MasterManifest;
use crate::config::ServerRegion;

/// Attempts for a request answered 5xx or not at all (503 is what the
/// registry's pg store answers while its database is unreachable).
const ATTEMPTS: u32 = 5;
/// Chunks buffered between the HTTP body and the parser.
const CHUNK_DEPTH: usize = 4;

/// A blob that can no longer be fetched: the registry pruned the manifest
/// that listed it (or a blob it names). The run is restarted from the new
/// `current` instead of being recorded as a failure.
#[derive(Debug)]
pub struct Gone(pub String);

impl std::fmt::Display for Gone {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} is gone from the registry", self.0)
    }
}

impl std::error::Error for Gone {}

#[derive(Clone)]
pub struct RegistryClient {
    base: String,
    http: reqwest::Client,
}

impl RegistryClient {
    pub fn new(base: &str) -> Self {
        let http = reqwest::Client::builder()
            .no_proxy()
            .connect_timeout(Duration::from_secs(10))
            .read_timeout(Duration::from_secs(60))
            .build()
            .unwrap_or_default();
        Self {
            base: base.trim_end_matches('/').to_string(),
            http,
        }
    }

    pub fn base(&self) -> &str {
        &self.base
    }

    /// GET with retries on connection errors and 5xx; other statuses are
    /// returned to the caller.
    async fn get(&self, path: &str) -> Result<reqwest::Response> {
        let url = format!("{}{}", self.base, path);
        let mut delay = Duration::from_millis(500);
        let mut attempt = 1;
        loop {
            let outcome = self.http.get(&url).send().await;
            let retry = match &outcome {
                Ok(resp) => resp.status().is_server_error(),
                Err(_) => true,
            };
            if !retry || attempt >= ATTEMPTS {
                return match outcome {
                    Ok(resp) if resp.status().is_server_error() => {
                        Err(anyhow!("GET {path}: registry answered {}", resp.status()))
                    }
                    Ok(resp) => Ok(resp),
                    Err(e) => Err(anyhow!("GET {path}: {e}")),
                };
            }
            tokio::time::sleep(delay).await;
            delay = (delay * 2).min(Duration::from_secs(8));
            attempt += 1;
        }
    }

    /// The registry's `blobStore` from `/health` (`fs` or `pg`).
    pub async fn blob_store(&self) -> Result<String> {
        let resp = self.get("/health").await?;
        if !resp.status().is_success() {
            anyhow::bail!("GET /health: registry answered {}", resp.status());
        }
        let health: serde_json::Value = resp.json().await.context("parsing /health")?;
        Ok(health["blobStore"].as_str().unwrap_or("fs").to_string())
    }

    /// The region's current manifest; `None` when it was never published.
    pub async fn current(&self, region: ServerRegion) -> Result<Option<MasterManifest>> {
        let resp = self
            .get(&format!("/v1/master/{}/current", region.as_str()))
            .await?;
        if resp.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(None);
        }
        if !resp.status().is_success() {
            anyhow::bail!("GET current: registry answered {}", resp.status());
        }
        let mut manifest: MasterManifest = resp.json().await.context("parsing current")?;
        if manifest.content_hash.is_empty() {
            manifest.content_hash = crate::api::internal::manifest_content_hash(&manifest.files);
        }
        Ok(Some(manifest))
    }

    /// Start streaming `blob/{sha256}`: the returned reader yields the body
    /// and fails at its end unless the bytes had `size` and `sha256`. A 404
    /// is a [`Gone`] error.
    pub async fn open_blob(
        &self,
        region: ServerRegion,
        sha256: &str,
        size: u64,
    ) -> Result<BlobReader> {
        let path = format!("/v1/master/{}/blob/{}", region.as_str(), sha256);
        let resp = self.get(&path).await?;
        if resp.status() == reqwest::StatusCode::NOT_FOUND {
            return Err(Gone(format!("blob {sha256}")).into());
        }
        if !resp.status().is_success() {
            anyhow::bail!("GET {path}: registry answered {}", resp.status());
        }
        let (tx, rx) = tokio::sync::mpsc::channel(CHUNK_DEPTH);
        let expected = sha256.to_string();
        tokio::spawn(pump(resp, expected, size, tx));
        Ok(BlobReader {
            rx,
            current: Bytes::new(),
        })
    }
}

/// Copy the body into the channel, then report a size or digest mismatch
/// (or a transport error) as the stream's last item.
async fn pump(
    mut resp: reqwest::Response,
    expected_sha: String,
    expected_size: u64,
    tx: tokio::sync::mpsc::Sender<std::io::Result<Bytes>>,
) {
    let mut hasher = sha2::Sha256::new();
    let mut size = 0u64;
    loop {
        match resp.chunk().await {
            Ok(Some(chunk)) => {
                hasher.update(&chunk);
                size += chunk.len() as u64;
                if tx.send(Ok(chunk)).await.is_err() {
                    return;
                }
            }
            Ok(None) => break,
            Err(e) => {
                let _ = tx
                    .send(Err(std::io::Error::other(format!("blob body: {e}"))))
                    .await;
                return;
            }
        }
    }
    let digest = hex::encode(hasher.finalize());
    if size != expected_size || digest != expected_sha {
        let _ = tx
            .send(Err(std::io::Error::other(format!(
                "blob {expected_sha}: got {size} bytes with digest {digest}"
            ))))
            .await;
    }
}

/// Blocking `Read` over a streamed blob (use from `spawn_blocking` only).
pub struct BlobReader {
    rx: tokio::sync::mpsc::Receiver<std::io::Result<Bytes>>,
    current: Bytes,
}

impl Read for BlobReader {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        while self.current.is_empty() {
            match self.rx.blocking_recv() {
                Some(Ok(chunk)) => self.current = chunk,
                Some(Err(e)) => return Err(e),
                None => return Ok(0),
            }
        }
        let n = buf.len().min(self.current.len());
        buf[..n].copy_from_slice(&self.current[..n]);
        self.current = self.current.slice(n..);
        Ok(n)
    }
}

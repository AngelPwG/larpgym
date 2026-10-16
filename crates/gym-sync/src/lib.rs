//! HTTP synchronization. All SQLite work runs on Tokio's blocking pool.
use anyhow::{ensure, Context, Result};
use gym_core::*;
use gym_storage::{Role, Store};
use serde::de::DeserializeOwned;
use std::{
    path::{Path, PathBuf},
    time::Duration,
};

#[derive(Clone)]
pub struct SyncClient {
    http: reqwest::Client,
    base: String,
    token: String,
}
async fn database<T: Send + 'static>(
    path: PathBuf,
    task: impl FnOnce(&mut Store) -> Result<T> + Send + 'static,
) -> Result<T> {
    tokio::task::spawn_blocking(move || task(&mut Store::open(path, Role::Client)?)).await?
}
impl SyncClient {
    pub fn new(base: &str, token: String) -> Result<Self> {
        let url = reqwest::Url::parse(base)?;
        ensure!(
            matches!(url.scheme(), "http" | "https")
                && url.username().is_empty()
                && url.password().is_none()
                && url.query().is_none()
                && url.fragment().is_none()
                && url.path() == "/",
            "Usa la URL raíz http(s) del backend, sin credenciales ni ruta"
        );
        ensure!(!token.is_empty(), "Falta el token");
        let http = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(2))
            .timeout(Duration::from_secs(8))
            .redirect(reqwest::redirect::Policy::none())
            .build().context("No se pudo inicializar HTTP/TLS; verifica los certificados CA del sistema (SSL_CERT_FILE)")?;
        Ok(Self {
            http,
            base: base.trim_end_matches('/').into(),
            token,
        })
    }
    async fn response<T: DeserializeOwned>(&self, response: reqwest::Response) -> Result<T> {
        let mut response = response.error_for_status().map_err(|e| {
            anyhow::anyhow!(
                "HTTP {}: comprueba autenticación y servidor",
                e.status().map_or(0, |s| s.as_u16())
            )
        })?;
        let mut bytes = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|_| anyhow::anyhow!("Respuesta interrumpida; se reintentará sin duplicar"))?
        {
            ensure!(
                bytes.len() + chunk.len() <= 16 * 1024 * 1024,
                "Respuesta del servidor demasiado grande"
            );
            bytes.extend_from_slice(&chunk);
        }
        serde_json::from_slice(&bytes).context("Respuesta de sincronización inválida")
    }
    pub async fn push(&self, operation: &Operation) -> Result<Outcome> {
        let response = self
            .http
            .post(format!("{}/api/v1/push", self.base))
            .bearer_auth(&self.token)
            .json(&PushRequest {
                operations: vec![operation.clone()],
            })
            .send()
            .await
            .map_err(|_| {
                anyhow::anyhow!("Sin conexión al backend; tus datos siguen guardados localmente")
            })?;
        let result: PushResponse = self.response(response).await?;
        ensure!(result.outcomes.len() == 1, "Confirmación de lote inválida");
        Ok(result.outcomes.into_iter().next().unwrap())
    }
    pub async fn pull(&self, cursor: u64) -> Result<Page> {
        let response = self
            .http
            .get(format!("{}/api/v1/changes", self.base))
            .bearer_auth(&self.token)
            .query(&[("cursor", cursor), ("limit", PAGE_SIZE as u64)])
            .send()
            .await
            .map_err(|_| anyhow::anyhow!("Sin conexión al backend; se reintentará"))?;
        self.response(response).await
    }
    pub async fn synchronize(&self, path: impl AsRef<Path>) -> Result<()> {
        let path = path.as_ref().to_owned();
        let _guard = database(path.clone(), |s| s.sync_lock()).await?;
        // Pull does not rebase user edits. It can adopt untouched catalogue defaults on a new device.
        self.download(&path).await?;
        for _ in 0..500 {
            let op = database(path.clone(), |s| s.prepare_next()).await?;
            let Some(op) = op else { break };
            let result = self.push(&op).await?;
            database(path.clone(), move |s| s.acknowledge(&op, &result)).await?;
        }
        self.download(&path).await?;
        database(path, |s| s.mark_synced()).await
    }
    async fn download(&self, path: &Path) -> Result<()> {
        let path = path.to_owned();
        loop {
            let cursor = database(path.clone(), |s| s.cursor()).await?;
            let page = self.pull(cursor).await?;
            let more = page.has_more;
            database(path.clone(), move |s| s.apply_page(&page)).await?;
            if !more {
                break;
            }
            tokio::task::yield_now().await;
        }
        Ok(())
    }
}

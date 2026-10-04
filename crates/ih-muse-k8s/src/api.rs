//! Read-only access to the Kubernetes API with the pod's service account.
//!
//! In a pod: the API host from `KUBERNETES_SERVICE_HOST`/`_PORT`, the token
//! and CA from `/var/run/secrets/kubernetes.io/serviceaccount`. The token is
//! read on every request (projected tokens rotate) and never appears in an
//! error. Tests point the client at a loopback HTTP server instead.

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::de::DeserializeOwned;

use crate::model::{
    Deployment, KubeEvent, List, Namespace, Node, NodeMetrics, Pod, PodMetrics, ReplicaSet,
    StatefulSet,
};

/// Where a pod's service account credentials are mounted.
pub const SERVICE_ACCOUNT_DIR: &str = "/var/run/secrets/kubernetes.io/serviceaccount";
/// Largest API answer the Muse reads (a namespace's pods, all nodes).
const MAX_RESPONSE_BYTES: usize = 64 * 1024 * 1024;
const MAX_TOKEN_BYTES: u64 = 16 * 1024;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(15);

/// Why an API read failed. Messages never contain the token.
#[derive(Debug, thiserror::Error)]
pub enum ApiError {
    #[error("Kubernetes API configuration: {0}")]
    Config(String),
    #[error("Kubernetes API {path}: HTTP {status}")]
    Status { path: String, status: u16 },
    #[error("Kubernetes API {path}: {message}")]
    Transport { path: String, message: String },
    #[error("Kubernetes API {path}: answer is not the expected JSON: {message}")]
    Decode { path: String, message: String },
}

/// How to reach the API: base URL, bearer token file and trusted CA.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ApiConfig {
    /// `https://<host>:<port>`, or loopback `http://127.0.0.1:<port>` in tests.
    pub base_url: String,
    /// Bearer token file; `None` sends no token (tests only).
    pub token_file: Option<PathBuf>,
    /// PEM CA bundle that signs the API server certificate.
    pub ca_file: Option<PathBuf>,
}

impl ApiConfig {
    /// The in-cluster configuration, with optional overrides. Without
    /// `api_url` the host comes from `KUBERNETES_SERVICE_HOST` and
    /// `KUBERNETES_SERVICE_PORT` (default 443); without a token or CA file
    /// the service account's mounted ones are used when the API is HTTPS.
    pub fn resolve(
        api_url: Option<&str>,
        token_file: Option<&Path>,
        ca_file: Option<&Path>,
        env: impl Fn(&str) -> Option<String>,
    ) -> Result<Self, ApiError> {
        let base_url = match api_url {
            Some(url) => url.trim().trim_end_matches('/').to_string(),
            None => {
                let host = env("KUBERNETES_SERVICE_HOST")
                    .filter(|host| !host.is_empty())
                    .ok_or_else(|| {
                        ApiError::Config(
                            "KUBERNETES_SERVICE_HOST is not set: run in a pod or pass --api-url"
                                .into(),
                        )
                    })?;
                let port = env("KUBERNETES_SERVICE_PORT").filter(|port| !port.is_empty());
                let host = if host.contains(':') {
                    format!("[{host}]")
                } else {
                    host
                };
                format!("https://{host}:{}", port.as_deref().unwrap_or("443"))
            }
        };
        let https = base_url.starts_with("https://");
        let loopback =
            base_url.starts_with("http://127.0.0.1") || base_url.starts_with("http://localhost");
        if !https && !loopback {
            return Err(ApiError::Config(
                "the API URL must use HTTPS (or loopback HTTP in tests)".into(),
            ));
        }
        let mounted = |name: &str| Path::new(SERVICE_ACCOUNT_DIR).join(name);
        let token_file = token_file
            .map(Path::to_path_buf)
            .or_else(|| (api_url.is_none() || https).then(|| mounted("token")));
        let ca_file = ca_file
            .map(Path::to_path_buf)
            .or_else(|| https.then(|| mounted("ca.crt")));
        Ok(Self {
            base_url,
            token_file,
            ca_file,
        })
    }

    /// The API host, used as the Muse instance when no cluster name is set.
    pub fn host(&self) -> String {
        let rest = self.base_url.split("://").nth(1).unwrap_or(&self.base_url);
        let authority = rest.split('/').next().unwrap_or(rest);
        match authority.strip_prefix('[') {
            Some(v6) => v6.split(']').next().unwrap_or(v6).to_string(),
            None => authority
                .rsplit_once(':')
                .map_or(authority, |(host, _)| host)
                .to_string(),
        }
    }
}

/// A read-only Kubernetes API client.
#[derive(Clone, Debug)]
pub struct KubeApi {
    config: ApiConfig,
    client: reqwest::Client,
}

impl KubeApi {
    pub fn new(config: ApiConfig) -> Result<Self, ApiError> {
        let mut builder = reqwest::Client::builder().timeout(REQUEST_TIMEOUT);
        if let Some(ca_file) = &config.ca_file {
            let pem = std::fs::read(ca_file).map_err(|error| {
                ApiError::Config(format!("cannot read CA {}: {error}", ca_file.display()))
            })?;
            let certificate = reqwest::Certificate::from_pem(&pem).map_err(|_| {
                ApiError::Config(format!("CA {} is not valid PEM", ca_file.display()))
            })?;
            // Trust only the cluster CA, not the public web roots.
            builder = builder
                .tls_built_in_root_certs(false)
                .add_root_certificate(certificate);
        }
        let client = builder
            .build()
            .map_err(|error| ApiError::Config(error.to_string()))?;
        Ok(Self { config, client })
    }

    pub fn config(&self) -> &ApiConfig {
        &self.config
    }

    fn token(&self) -> Result<Option<String>, ApiError> {
        let Some(path) = &self.config.token_file else {
            return Ok(None);
        };
        let unreadable = || ApiError::Config(format!("cannot read API token {}", path.display()));
        let size = std::fs::metadata(path).map_err(|_| unreadable())?.len();
        if size == 0 || size > MAX_TOKEN_BYTES {
            return Err(ApiError::Config(format!(
                "API token {} is empty or too large",
                path.display()
            )));
        }
        let token = std::fs::read_to_string(path).map_err(|_| unreadable())?;
        Ok(Some(token.trim().to_string()))
    }

    /// GETs `path` (for example `/api/v1/nodes`) and decodes the JSON answer.
    pub async fn get<T: DeserializeOwned>(&self, path: &str) -> Result<T, ApiError> {
        let transport = |message: String| ApiError::Transport {
            path: path.into(),
            message,
        };
        let mut request = self
            .client
            .get(format!("{}{path}", self.config.base_url))
            .header(reqwest::header::ACCEPT, "application/json");
        if let Some(token) = self.token()? {
            request = request.bearer_auth(token);
        }
        // `without_url`: the error text must not echo request details.
        let mut response = request
            .send()
            .await
            .map_err(|error| transport(error.without_url().to_string()))?;
        if !response.status().is_success() {
            return Err(ApiError::Status {
                path: path.into(),
                status: response.status().as_u16(),
            });
        }
        let mut body = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|error| transport(error.without_url().to_string()))?
        {
            if body.len() + chunk.len() > MAX_RESPONSE_BYTES {
                return Err(transport(format!(
                    "answer larger than {MAX_RESPONSE_BYTES} bytes"
                )));
            }
            body.extend_from_slice(&chunk);
        }
        serde_json::from_slice(&body).map_err(|error| ApiError::Decode {
            path: path.into(),
            message: error.to_string(),
        })
    }

    pub async fn nodes(&self) -> Result<Vec<Node>, ApiError> {
        Ok(self.get::<List<Node>>("/api/v1/nodes").await?.items)
    }

    pub async fn pods(&self, namespace: &str) -> Result<Vec<Pod>, ApiError> {
        Ok(self
            .get::<List<Pod>>(&format!("/api/v1/namespaces/{namespace}/pods"))
            .await?
            .items)
    }

    /// Every namespace's pods (needs `list pods` cluster-wide).
    pub async fn all_pods(&self) -> Result<Vec<Pod>, ApiError> {
        Ok(self.get::<List<Pod>>("/api/v1/pods").await?.items)
    }

    /// Every namespace, with its name and UID.
    pub async fn namespaces(&self) -> Result<Vec<Namespace>, ApiError> {
        Ok(self
            .get::<List<Namespace>>("/api/v1/namespaces")
            .await?
            .items)
    }

    /// Every namespace's pod usage from metrics-server.
    pub async fn all_pod_metrics(&self) -> Result<Vec<PodMetrics>, ApiError> {
        Ok(self
            .get::<List<PodMetrics>>("/apis/metrics.k8s.io/v1beta1/pods")
            .await?
            .items)
    }

    /// Deployments of `namespace`, or of every namespace when `None`.
    pub async fn deployments(&self, namespace: Option<&str>) -> Result<Vec<Deployment>, ApiError> {
        Ok(self
            .get::<List<Deployment>>(&apps_path(namespace, "deployments"))
            .await?
            .items)
    }

    /// StatefulSets of `namespace`, or of every namespace when `None`.
    pub async fn statefulsets(
        &self,
        namespace: Option<&str>,
    ) -> Result<Vec<StatefulSet>, ApiError> {
        Ok(self
            .get::<List<StatefulSet>>(&apps_path(namespace, "statefulsets"))
            .await?
            .items)
    }

    /// ReplicaSets (each Deployment revision's template).
    pub async fn replicasets(&self, namespace: Option<&str>) -> Result<Vec<ReplicaSet>, ApiError> {
        Ok(self
            .get::<List<ReplicaSet>>(&apps_path(namespace, "replicasets"))
            .await?
            .items)
    }

    /// Kubernetes `Warning` events still kept by the API (about an hour).
    pub async fn warning_events(
        &self,
        namespace: Option<&str>,
    ) -> Result<Vec<KubeEvent>, ApiError> {
        let base = match namespace {
            Some(namespace) => format!("/api/v1/namespaces/{namespace}/events"),
            None => "/api/v1/events".to_string(),
        };
        Ok(self
            .get::<List<KubeEvent>>(&format!("{base}?fieldSelector=type%3DWarning"))
            .await?
            .items)
    }

    pub async fn namespace_uid(&self, namespace: &str) -> Result<String, ApiError> {
        let path = format!("/api/v1/namespaces/{namespace}");
        let uid = self.get::<Namespace>(&path).await?.metadata.uid;
        if uid.is_empty() {
            return Err(ApiError::Decode {
                path,
                message: "namespace has no uid".into(),
            });
        }
        Ok(uid)
    }

    pub async fn node_metrics(&self) -> Result<Vec<NodeMetrics>, ApiError> {
        Ok(self
            .get::<List<NodeMetrics>>("/apis/metrics.k8s.io/v1beta1/nodes")
            .await?
            .items)
    }

    pub async fn pod_metrics(&self, namespace: &str) -> Result<Vec<PodMetrics>, ApiError> {
        Ok(self
            .get::<List<PodMetrics>>(&format!(
                "/apis/metrics.k8s.io/v1beta1/namespaces/{namespace}/pods"
            ))
            .await?
            .items)
    }
}

/// `/apis/apps/v1/[namespaces/{ns}/]{resource}`.
fn apps_path(namespace: Option<&str>, resource: &str) -> String {
    match namespace {
        Some(namespace) => format!("/apis/apps/v1/namespaces/{namespace}/{resource}"),
        None => format!("/apis/apps/v1/{resource}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env(pairs: &'static [(&'static str, &'static str)]) -> impl Fn(&str) -> Option<String> {
        move |name| {
            pairs
                .iter()
                .find(|(key, _)| *key == name)
                .map(|(_, value)| value.to_string())
        }
    }

    #[test]
    fn in_cluster_configuration_uses_the_service_account_mount() {
        let config = ApiConfig::resolve(
            None,
            None,
            None,
            env(&[
                ("KUBERNETES_SERVICE_HOST", "10.43.0.1"),
                ("KUBERNETES_SERVICE_PORT", "443"),
            ]),
        )
        .unwrap();
        assert_eq!(config.base_url, "https://10.43.0.1:443");
        assert_eq!(
            config.token_file,
            Some(Path::new(SERVICE_ACCOUNT_DIR).join("token"))
        );
        assert_eq!(
            config.ca_file,
            Some(Path::new(SERVICE_ACCOUNT_DIR).join("ca.crt"))
        );
        assert_eq!(config.host(), "10.43.0.1");

        let v6 = ApiConfig::resolve(
            None,
            None,
            None,
            env(&[("KUBERNETES_SERVICE_HOST", "fd00::1")]),
        )
        .unwrap();
        assert_eq!(v6.base_url, "https://[fd00::1]:443");
        assert_eq!(v6.host(), "fd00::1");
    }

    #[test]
    fn overrides_and_transport_rules() {
        assert!(matches!(
            ApiConfig::resolve(None, None, None, env(&[])),
            Err(ApiError::Config(_))
        ));
        assert!(ApiConfig::resolve(Some("http://api.example:6443"), None, None, env(&[])).is_err());
        let test = ApiConfig::resolve(Some("http://127.0.0.1:9/"), None, None, env(&[])).unwrap();
        assert_eq!(test.base_url, "http://127.0.0.1:9");
        assert_eq!(
            (test.token_file, test.ca_file),
            (None, None),
            "loopback tests need no credentials"
        );
        let explicit = ApiConfig::resolve(
            Some("https://api.example:6443"),
            Some(Path::new("/tmp/t")),
            Some(Path::new("/tmp/ca")),
            env(&[]),
        )
        .unwrap();
        assert_eq!(explicit.host(), "api.example");
        assert_eq!(explicit.token_file.as_deref(), Some(Path::new("/tmp/t")));
    }
}

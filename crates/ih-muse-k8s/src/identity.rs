//! Who this Muse is, in the fields where the macOS Muse states it.
//!
//! The macOS Muse writes `ih-muse-macos:<host>` as provenance `source_id`,
//! its crate version as `source_revision`, and `ih-muse-macos:<host>:<time>`
//! as the delivery id. This Muse writes `ih-k8s-muse:<instance>`, its crate
//! version and `ih-k8s-muse:<instance>:<time>`. The `ih-k8s-muse` prefix is
//! the Python Muse's source id, kept so the producer stays recognisable;
//! `<instance>` is the configured cluster name, else the API host. Poet lists
//! the producer as `ih-k8s-muse:<instance>@<version>` (Fleet "producers").
//! The kind, `k8s`, is the owning `muse_kind` of the dashboard definition.

/// The Muse kind: owner of the `k8s.*` dashboard definitions.
pub const MUSE_KIND: &str = "k8s";
/// The source id prefix (the reference Python Muse's whole source id).
pub const SOURCE_PREFIX: &str = "ih-k8s-muse";
/// This Muse's version, sent as provenance `source_revision`.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// The Muse instance this process speaks for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MuseIdentity {
    /// Cluster name if configured, else the Kubernetes API host.
    pub instance: String,
}

impl MuseIdentity {
    /// `cluster_name` wins over the API host when given and not blank.
    pub fn new(cluster_name: Option<&str>, api_host: &str) -> Self {
        let instance = cluster_name
            .map(str::trim)
            .filter(|name| !name.is_empty())
            .unwrap_or(api_host);
        Self {
            instance: instance.to_string(),
        }
    }

    /// Provenance `source_id`: `ih-k8s-muse:<instance>`.
    pub fn source_id(&self) -> String {
        format!("{SOURCE_PREFIX}:{}", self.instance)
    }

    /// Provenance `source_revision`: the crate version.
    pub fn source_revision(&self) -> String {
        VERSION.into()
    }

    /// Delivery id of the batch observed at `now`; stable across retries
    /// and Poets, so a resend is stored once.
    pub fn delivery_id(&self, now_unix_nano: u64) -> String {
        format!("{}:{now_unix_nano}", self.source_id())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_cluster_name_wins_over_the_api_host() {
        let named = MuseIdentity::new(Some("k-lab"), "10.43.0.1");
        assert_eq!(named.source_id(), "ih-k8s-muse:k-lab");
        assert_eq!(named.delivery_id(7), "ih-k8s-muse:k-lab:7");
        assert_eq!(named.source_revision(), VERSION);
        assert_eq!(MuseIdentity::new(None, "10.43.0.1").instance, "10.43.0.1");
        assert_eq!(
            MuseIdentity::new(Some("  "), "10.43.0.1").instance,
            "10.43.0.1"
        );
    }
}

use crate::connection::{Connection, Response};
use anyhow::anyhow;
use log::trace;
use maplit::hashmap;
use rustls::{OwnedTrustAnchor, RootCertStore};
use std::sync::Arc;
use url::Url;

pub struct Client {
    rustls_config: Arc<rustls::ClientConfig>,
}

impl Client {
    pub fn get(&self, url: Url) -> anyhow::Result<Response> {
        let headers = hashmap! {
            ":method".to_owned() => "GET".to_owned(),
            ":scheme".to_owned() => url.scheme().to_owned(),
            ":authority".to_owned() => format!(
                "{}:{}",
                url.host_str().ok_or_else(|| anyhow!("No host in URL"))?,
                url.port_or_known_default().ok_or_else(|| anyhow!("No port for URL"))?,
            ),
            ":path".to_owned() => url.path().to_owned(),
        };
        trace!("GET {} {:#?}", url, headers);
        let connection = Connection::connect(url, self.rustls_config.clone())?;
        let response = connection.request(headers, Vec::new());
        trace!("Response: {:#?}", response);
        Ok(response)
    }
}

impl Default for Client {
    fn default() -> Self {
        let mut root_store = RootCertStore::empty();
        root_store.add_server_trust_anchors(webpki_roots::TLS_SERVER_ROOTS.0.iter().map(|ta| {
            OwnedTrustAnchor::from_subject_spki_name_constraints(
                ta.subject,
                ta.spki,
                ta.name_constraints,
            )
        }));
        let mut config = rustls::ClientConfig::builder()
            .with_safe_defaults()
            .with_root_certificates(root_store)
            .with_no_client_auth();
        config.alpn_protocols = vec![vec![b'h', b'2']];
        Self {
            rustls_config: Arc::new(config),
        }
    }
}

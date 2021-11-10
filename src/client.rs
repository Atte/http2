use crate::connection::{Connection, Response};
use anyhow::anyhow;
use std::sync::Arc;
use tokio_rustls::{
    rustls::{ClientConfig, OwnedTrustAnchor, RootCertStore},
    TlsConnector,
};
use url::Url;

pub struct Client {
    connector: TlsConnector,
}

impl Client {
    pub async fn get(&self, url: Url) -> anyhow::Result<Response> {
        let headers = vec![
            (":method".to_owned(), "GET".to_owned()),
            (":scheme".to_owned(), url.scheme().to_owned()),
            (":path".to_owned(), url.path().to_owned()),
            (
                ":authority".to_owned(),
                if let Some(port) = url.port() {
                    format!(
                        "{}:{}",
                        url.host_str().ok_or_else(|| anyhow!("No host in URL"))?,
                        port,
                    )
                } else {
                    url.host_str()
                        .ok_or_else(|| anyhow!("No host in URL"))?
                        .to_owned()
                },
            ),
        ];
        let connection = Connection::connect(url, &self.connector).await?;
        let response = connection.request(headers, Vec::new()).await?;
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
        let mut config = ClientConfig::builder()
            .with_safe_defaults()
            .with_root_certificates(root_store)
            .with_no_client_auth();
        config.alpn_protocols = vec![vec![b'h', b'2']];
        Self {
            connector: Arc::new(config).into(),
        }
    }
}

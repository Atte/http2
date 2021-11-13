use crate::{connection::Connection, request::Request, response::Response};
use std::sync::Arc;
use tokio_rustls::{
    rustls::{ClientConfig, OwnedTrustAnchor, RootCertStore},
    TlsConnector,
};

pub struct Client {
    connector: TlsConnector,
}

impl Client {
    pub async fn request(&self, request: Request) -> anyhow::Result<Response> {
        let connection = Connection::connect(&request.url, &self.connector).await?;
        let response = connection.request(request).await?;
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

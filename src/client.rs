use crate::{connection::Connection, request::Request, response::Response};
use std::{collections::HashMap, sync::Arc};
use tokio::sync::Mutex;
use tokio_rustls::{
    rustls::{ClientConfig, OwnedTrustAnchor, RootCertStore},
    TlsConnector,
};
use url::Origin;

pub struct Client {
    connector: TlsConnector,
    // TODO: no Mutex?
    connections: Mutex<HashMap<Origin, Connection>>,
}

impl Client {
    pub async fn request(&self, request: Request) -> anyhow::Result<Response> {
        let origin = request.url.origin();
        let mut connections = self.connections.lock().await;
        if connections.get(&origin).is_none() {
            connections.insert(
                origin.clone(),
                Connection::connect(&request.url, &self.connector).await?,
            );
        }
        Ok(connections.get(&origin).unwrap().request(request).await?)
    }
}

impl Default for Client {
    #[must_use]
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
            connections: Default::default(),
        }
    }
}

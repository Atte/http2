use crate::{connection::Connection, frame::*, stream::Stream, types::*};
use url::Url;

pub struct Client;

impl Client {
    pub async fn get(url: Url) -> anyhow::Result<Vec<u8>> {
        let mut connection = Connection::connect(url.socket_addrs(|| None)?[0]).await?;
        Ok(Vec::new())
    }
}

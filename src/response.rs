use crate::types::Headers;
use bytes::Bytes;

#[derive(Debug, Clone)]
pub struct Response {
    pub request_id: usize,
    pub headers: Headers,
    pub body: Bytes,
}

impl Response {
    pub fn header(&self, key: impl AsRef<str>) -> Option<&str> {
        let key = key.as_ref();
        self.headers
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_ref())
    }

    pub fn status(&self) -> u8 {
        self.header(":status")
            .expect("no status in response")
            .parse()
            .expect("non-number status")
    }
}

use crate::types::Headers;
use bytes::Bytes;
use std::borrow::Cow;

#[derive(Debug, Clone)]
pub struct Response {
    pub request_id: usize,
    pub headers: Headers,
    pub body: Bytes,
}

impl Response {
    pub fn headers<'a>(&'a self, key: &'a str) -> impl Iterator<Item = &'a str> {
        // https://httpwg.org/specs/rfc7540.html#HttpHeaders
        // response headers MUST already be lowercase by spec, so only need to lower the user input
        let key = key.to_lowercase();
        self.headers
            .iter()
            .filter(move |(k, _)| k == &key)
            .map(|(_, v)| v.as_ref())
    }

    pub fn header<'a>(&'a self, key: &'a str) -> Option<&'a str> {
        self.headers(key).next()
    }

    pub fn status(&self) -> u16 {
        self.header(":status")
            .expect("no status in response")
            .parse()
            .expect("non-number status")
    }

    pub fn text(&self) -> Cow<'_, str> {
        String::from_utf8_lossy(&self.body)
    }

    #[cfg(feature = "json")]
    pub fn json<'a, T>(&'a self) -> serde_json::Result<T>
    where
        T: serde::Deserialize<'a>,
    {
        serde_json::from_slice(&self.body)
    }
}

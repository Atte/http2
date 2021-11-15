use crate::types::Headers;
use bytes::Bytes;
use std::borrow::Cow;

#[derive(Debug, Clone)]
pub struct Response {
    pub headers: Headers,
    pub body: Bytes,
}

impl Response {
    pub fn headers<'a>(&'a self, key: &'a str) -> Option<&Vec<String>> {
        // response headers MUST already be lowercase by spec, so only need to lower the user input
        self.headers.get(&key.to_lowercase())
    }

    #[inline]
    pub fn header<'a>(&'a self, key: &'a str) -> Option<&'a str> {
        self.headers(key)
            .and_then(|values| values.first().map(String::as_ref))
    }

    pub fn status(&self) -> u16 {
        self.header(":status")
            .expect("no status in response")
            .parse()
            .expect("non-number status")
    }

    #[inline]
    pub fn ok(&self) -> bool {
        (200..300).contains(&self.status())
    }

    #[inline]
    pub fn text(&self) -> Cow<'_, str> {
        String::from_utf8_lossy(&self.body)
    }

    #[cfg(feature = "json")]
    #[inline]
    pub fn json<'a, T>(&'a self) -> serde_json::Result<T>
    where
        T: serde::Deserialize<'a>,
    {
        serde_json::from_slice(&self.body)
    }
}

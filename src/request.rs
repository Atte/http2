use crate::types::Headers;
use bytes::Bytes;
use std::sync::atomic::{AtomicUsize, Ordering};
use url::Url;

static REQUEST_ID: AtomicUsize = AtomicUsize::new(1);

#[derive(Debug, derive_more::Display)]
pub enum Method {
    #[display(fmt = "GET")]
    Get,
    #[display(fmt = "POST")]
    Post,
    #[display(fmt = "PUT")]
    Put,
    #[display(fmt = "DELETE")]
    Delete,
    #[display(fmt = "HEAD")]
    Head,
    #[display(fmt = "PATCH")]
    Patch,
    #[display(fmt = "OPTIONS")]
    Options,
    #[display(fmt = "{}", _0)]
    Other(String),
}

#[derive(Debug, Clone)]
pub struct Request {
    pub id: usize,
    pub url: Url,
    pub headers: Headers,
    pub body: Bytes,
}

impl Request {
    pub fn new(method: Method, url: Url, headers: Option<Headers>, body: impl Into<Bytes>) -> Self {
        let mut full_headers = vec![
            (":method".to_owned(), method.to_string()),
            (":scheme".to_owned(), url.scheme().to_owned()),
            (":path".to_owned(), url.path().to_owned()),
            (
                ":authority".to_owned(),
                if let Some(port) = url.port() {
                    format!("{}:{}", url.host_str().expect("URL cannot be a base"), port)
                } else {
                    url.host_str().expect("URL cannot be a base").to_owned()
                },
            ),
        ];
        if let Some(headers) = headers {
            full_headers.extend(headers);
        }
        Self {
            id: REQUEST_ID.fetch_add(1, Ordering::SeqCst),
            url,
            headers: full_headers,
            body: body.into(),
        }
    }

    #[inline]
    pub fn get(url: Url, headers: Option<Headers>) -> Self {
        Self::new(Method::Get, url, headers, Bytes::new())
    }

    #[inline]
    pub fn post(url: Url, headers: Option<Headers>, body: impl Into<Bytes>) -> Self {
        Self::new(Method::Post, url, headers, body)
    }
}

use crate::{
    connection::ConnectionState, flags::*, frame::*, stream_coordinator::StreamCoordinator,
    types::Headers,
};
use bytes::Bytes;
use std::{
    fmt,
    sync::atomic::{AtomicUsize, Ordering},
};
use url::Url;

static REQUEST_ID: AtomicUsize = AtomicUsize::new(1);

#[derive(Debug, Clone)]
pub enum Method {
    Get,
    Post,
    Put,
    Delete,
    Head,
    Patch,
    Options,
    Other(String),
}

impl AsRef<str> for Method {
    fn as_ref(&self) -> &str {
        match self {
            Self::Get => "GET",
            Self::Post => "POST",
            Self::Put => "PUT",
            Self::Delete => "DELETE",
            Self::Head => "HEAD",
            Self::Patch => "PATCH",
            Self::Options => "OPTIONS",
            Self::Other(s) => s.as_ref(),
        }
    }
}

impl fmt::Display for Method {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> Result<(), fmt::Error> {
        f.write_str(self.as_ref())
    }
}

#[derive(Debug, Clone)]
pub struct Request {
    pub id: usize,
    pub url: Url,
    pub method: Method,
    pub headers: Headers,
    pub body: Bytes,
}

impl Request {
    pub fn get(url: Url) -> Self {
        Self {
            id: REQUEST_ID.fetch_add(1, Ordering::SeqCst),
            url,
            method: Method::Get,
            headers: Headers::default(),
            body: Bytes::default(),
        }
    }

    pub fn post(url: Url, body: Bytes) -> Self {
        Self {
            id: REQUEST_ID.fetch_add(1, Ordering::SeqCst),
            url,
            method: Method::Post,
            headers: Headers::default(),
            body,
        }
    }

    #[cfg(feature = "json")]
    pub fn post_json<T>(url: Url, body: &T) -> serde_json::Result<Self>
    where
        T: serde::Serialize,
    {
        Ok(Self {
            id: REQUEST_ID.fetch_add(1, Ordering::SeqCst),
            url,
            method: Method::Post,
            headers: vec![("content-type".to_owned(), "application/json".to_owned())],
            body: Bytes::from(serde_json::to_vec(body)?),
        })
    }

    pub fn write_into(self, state: &mut ConnectionState, streams: &mut StreamCoordinator) {
        let authority = if let Some(port) = self.url.port() {
            format!(
                "{}:{}",
                self.url.host().expect("URL cannot be a base"),
                port
            )
        } else {
            self.url.host().expect("URL cannot be a base").to_string()
        };
        let pseudo_headers: [(&[u8], &[u8]); 4] = [
            (b":method", self.method.as_ref().as_bytes()),
            (b":scheme", self.url.scheme().as_bytes()),
            (b":path", self.url.path().as_bytes()),
            (b":authority", authority.as_bytes()),
        ];
        // header names MUST be lowercase
        let headers: Vec<(String, String)> = self
            .headers
            .into_iter()
            .map(|(k, v)| (k.to_lowercase(), v))
            .collect();

        let stream = streams.create_mut();
        stream.request_id = self.id;

        FramePayload::Headers {
            dependency: None,
            exclusive_dependency: None,
            weight: None,
            fragment: state
                .header_encoder
                .encode(
                    // pseudo-headers MUST be first
                    pseudo_headers
                        .into_iter()
                        .chain(headers.iter().map(|(k, v)| (k.as_bytes(), v.as_bytes()))),
                )
                .into(),
        }
        .write_into(
            &mut state.write_buf,
            Some(stream),
            if self.body.is_empty() {
                HeadersFlags::END_STREAM | HeadersFlags::END_HEADERS
            } else {
                HeadersFlags::END_HEADERS
            },
        );

        if !self.body.is_empty() {
            FramePayload::Data { data: self.body }.write_into(
                &mut state.write_buf,
                Some(stream),
                DataFlags::END_STREAM,
            );
        }
    }
}

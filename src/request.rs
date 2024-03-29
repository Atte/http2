use crate::{
    connection::ConnectionState, flags::*, frame::*, response::Response,
    stream_coordinator::StreamCoordinator, types::*,
};
use bytes::Bytes;
use maplit::hashmap;
use std::fmt;
use tokio::sync::oneshot;
use url::Url;

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
    #[inline]
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> Result<(), fmt::Error> {
        f.write_str(self.as_ref())
    }
}

#[derive(Debug, Clone)]
#[must_use]
pub struct Request {
    pub url: Url,
    pub method: Method,
    pub headers: Headers,
    pub body: Bytes,
}

impl Request {
    pub fn new(method: Method, url: Url, headers: Headers, body: impl Into<Bytes>) -> Self {
        Self {
            url,
            method,
            headers,
            body: body.into(),
        }
    }

    #[inline]
    pub fn head(url: Url) -> Self {
        Self::new(Method::Head, url, Headers::new(), Bytes::new())
    }

    #[inline]
    pub fn get(url: Url) -> Self {
        Self::new(Method::Get, url, Headers::new(), Bytes::new())
    }

    #[inline]
    pub fn delete(url: Url) -> Self {
        Self::new(Method::Delete, url, Headers::new(), Bytes::new())
    }

    #[cfg(feature = "json")]
    pub fn post_json<T>(url: Url, body: &T) -> serde_json::Result<Self>
    where
        T: serde::Serialize,
    {
        Ok(Self::new(
            Method::Post,
            url,
            hashmap! { "content-type".to_owned() => vec!["application/json".to_owned()] },
            serde_json::to_vec(body)?,
        ))
    }

    #[cfg(feature = "json")]
    pub fn put_json<T>(url: Url, body: &T) -> serde_json::Result<Self>
    where
        T: serde::Serialize,
    {
        Ok(Self::new(
            Method::Put,
            url,
            hashmap! { "content-type".to_owned() => vec!["application/json".to_owned()] },
            serde_json::to_vec(body)?,
        ))
    }

    #[cfg(feature = "json")]
    pub fn patch_json<T>(url: Url, body: &T) -> serde_json::Result<Self>
    where
        T: serde::Serialize,
    {
        Ok(Self::new(
            Method::Patch,
            url,
            hashmap! { "content-type".to_owned() => vec!["application/json".to_owned()] },
            serde_json::to_vec(body)?,
        ))
    }

    pub fn redirect(&self, response: &Response) -> Option<Self> {
        let (method, body) = match response.status() {
            // change method to GET
            301 | 302 | 303 => (Method::Get, Bytes::new()),
            // use the same method
            307 | 308 => (self.method.clone(), self.body.clone()),
            _ => {
                return None;
            }
        };

        let location = response
            .header("location")
            .and_then(|location| self.url.join(location).ok())?;

        Some(Self::new(method, location, self.headers.clone(), body))
    }

    pub(crate) fn write_into(
        self,
        state: &mut ConnectionState,
        streams: &mut StreamCoordinator,
        response_tx: oneshot::Sender<Response>,
    ) -> Result<(), RequestError> {
        let path = if let Some(query) = self.url.query() {
            format!("{}?{}", self.url.path(), query)
        } else {
            self.url.path().to_owned()
        };
        let authority = if let Some(port) = self.url.port() {
            format!(
                "{}:{}",
                self.url.host().ok_or(RequestError::AuthorityCannotBeBase)?,
                port
            )
        } else {
            self.url
                .host()
                .ok_or(RequestError::AuthorityCannotBeBase)?
                .to_string()
        };
        let pseudo_headers: [(&[u8], &[u8]); 4] = [
            (b":method", self.method.as_ref().as_bytes()),
            (b":scheme", self.url.scheme().as_bytes()),
            (b":path", path.as_bytes()),
            (b":authority", authority.as_bytes()),
        ];
        let headers: Vec<(String, String)> = self
            .headers
            .into_iter()
            // header names MUST be lowercase
            .flat_map(|(k, vs)| vs.into_iter().map(move |v| (k.to_lowercase(), v)))
            .collect();

        let stream = streams.create_mut().ok_or(RequestError::OutOfStreamIds)?;
        stream.response_tx = Some(response_tx);

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

        Ok(())
    }
}

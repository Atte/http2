#![warn(future_incompatible, nonstandard_style, rust_2018_idioms, unused)]
#![warn(clippy::pedantic)]
#![allow(
    clippy::doc_markdown,
    clippy::missing_errors_doc,
    clippy::missing_panics_doc,
    clippy::module_name_repetitions,
    clippy::wildcard_imports,
    clippy::similar_names,
    clippy::cast_possible_truncation, // TODO
    clippy::too_many_lines, // TODO
)]

mod client;
mod connection;
mod flags;
mod frame;
mod request;
mod response;
mod stream;
mod stream_coordinator;
mod types;

pub use bytes::Bytes;
pub use client::Client;
pub use request::{Method, Request};
pub use response::Response;
pub use url::Url;

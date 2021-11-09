use crate::{connection::Response, frame::Frame, types::*};
use log::warn;
use std::{collections::HashMap, io::Write, sync::Mutex};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum StreamState {
    Idle,
    ReservedLocal,
    ReservedRemote,
    Open,
    HalfClosedLocal,
    HalfClosedRemote,
    Closed,
}

pub struct Stream {
    pub id: NonZeroStreamId,
    pub request_headers: HashMap<String, String>,
    window_remaining: u64,
    state: StreamState,
    dependency: StreamId,
    weight: u8,
    headers_buffer: Vec<u8>,
    body_buffer: Vec<u8>,
    response_headers: HashMap<String, String>,
}

impl Stream {
    pub fn new(id: NonZeroStreamId, window_remaining: u64) -> Self {
        Self {
            id,
            request_headers: HashMap::new(),
            window_remaining,
            state: StreamState::Idle,
            dependency: 0,
            weight: 0,
            headers_buffer: Vec::new(),
            body_buffer: Vec::new(),
            response_headers: HashMap::new(),
        }
    }

    pub fn on_frame(
        &mut self,
        frame: Frame,
        writable: &Mutex<impl Write>,
        header_decoder: &mut hpack::Decoder,
    ) -> anyhow::Result<Option<Response>> {
        Ok(match frame {
            Frame::Data { .. } => {
                // TODO: proper flow control
                Frame::WindowUpdate {
                    stream: self.id.get(),
                    increment: U31_MAX,
                }
                .write_into(writable)?;
                Frame::WindowUpdate {
                    stream: 0,
                    increment: U31_MAX,
                }
                .write_into(writable)?;
                None
            }
            Frame::Headers {
                flags,
                dependency,
                weight,
                fragment,
                ..
            } => {
                if self.state == StreamState::Closed {
                    self.state = StreamState::Open;
                } else if self.state == StreamState::ReservedRemote {
                    self.state = StreamState::HalfClosedLocal;
                }

                if flags.contains(HeadersFlags::PRIORITY) {
                    self.dependency = dependency;
                    self.weight = weight;
                }

                self.headers_buffer.extend(fragment);
                if flags.contains(HeadersFlags::END_HEADERS) {
                    self.decode_headers(header_decoder);
                }

                match (
                    flags.contains(HeadersFlags::END_HEADERS),
                    flags.contains(HeadersFlags::END_STREAM),
                ) {
                    (true, true) => {
                        self.state = StreamState::Closed;
                        self.decode_headers(header_decoder);
                        Some(self.decode_response())
                    }
                    (true, false) => {
                        self.decode_headers(header_decoder);
                        None
                    }
                    (false, true) => {
                        self.state = StreamState::Closed;
                        None
                    }
                    (false, false) => None,
                }
            }
            Frame::Priority {
                dependency, weight, ..
            } => {
                self.dependency = dependency;
                self.weight = weight;
                None
            }
            Frame::ResetStream { error, .. } => {
                warn!("Reset stream: {:?}", error);
                self.state = StreamState::Closed;
                None
            }
            Frame::Settings { .. } => unreachable!("can't be sent to a stream"),
            Frame::PushPromise {
                promised_stream, ..
            } => {
                // TODO: handle pushes
                Frame::ResetStream {
                    stream: NonZeroStreamId::new(promised_stream)
                        .ok_or(FrameDecodeError::ZeroStreamId)?,
                    error: ErrorType::RefusedStream,
                }
                .write_into(writable)?;
                None
            }
            Frame::Ping { .. } => unreachable!("can't be sent to a stream"),
            Frame::GoAway { .. } => unreachable!("can't be sent to a stream"),
            Frame::WindowUpdate { increment, .. } => {
                self.window_remaining +=
                    self.window_remaining.saturating_add(increment.get() as u64);
                None
            }
            Frame::Continuation {
                flags, fragment, ..
            } => {
                self.headers_buffer.extend(fragment);
                if flags.contains(ContinuationFlags::END_HEADERS) {
                    self.decode_headers(header_decoder);
                    if self.state == StreamState::Closed {
                        Some(self.decode_response())
                    } else {
                        None
                    }
                } else {
                    None
                }
            }
        })
    }

    fn decode_headers(&mut self, header_decoder: &mut hpack::Decoder) {
        header_decoder
            .decode_with_cb(&self.headers_buffer, |key, value| {
                self.response_headers.insert(
                    String::from_utf8_lossy(&key).to_string(),
                    String::from_utf8_lossy(&value).to_string(),
                );
            })
            .expect("decode_with_cb");
        self.headers_buffer.clear();
    }

    fn decode_response(&self) -> Response {
        Response {
            request_headers: self.request_headers.clone(),
            headers: self.response_headers.clone(),
            body: self.body_buffer.clone(),
        }
    }
}

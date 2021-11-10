use crate::{connection::Response, frame::Frame, types::*};
use bytes::{BufMut, BytesMut};
use log::warn;

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
    pub request_headers: Vec<(String, String)>,
    window_remaining: u64,
    state: StreamState,
    dependency: Option<StreamId>,
    exclusive_dependency: Option<bool>,
    weight: Option<u8>,
    headers_buffer: BytesMut,
    body_buffer: BytesMut,
    response_headers: Vec<(String, String)>,
}

impl Stream {
    #[must_use]
    pub fn new(id: NonZeroStreamId, window_remaining: u64) -> Self {
        Self {
            id,
            request_headers: Vec::new(),
            window_remaining,
            state: StreamState::Idle,
            dependency: None,
            exclusive_dependency: None,
            weight: None,
            headers_buffer: BytesMut::with_capacity(16_384 * 2),
            body_buffer: BytesMut::with_capacity(16_384 * 2),
            response_headers: Vec::new(),
        }
    }

    pub fn on_frame(
        &mut self,
        frame: Frame,
        send_buffer: &mut impl BufMut,
        header_decoder: &mut hpack::Decoder,
    ) -> anyhow::Result<Option<Response>> {
        Ok(match frame {
            Frame::Data { .. } => {
                // TODO: proper flow control
                Frame::WindowUpdate {
                    stream: self.id.get(),
                    increment: U31_MAX,
                }
                .into_bytes(send_buffer);
                Frame::WindowUpdate {
                    stream: 0,
                    increment: U31_MAX,
                }
                .into_bytes(send_buffer);
                None
            }
            Frame::Headers {
                flags,
                dependency,
                exclusive_dependency,
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
                    self.exclusive_dependency = exclusive_dependency;
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
                dependency,
                exclusive_dependency,
                weight,
                ..
            } => {
                self.dependency = Some(dependency);
                self.exclusive_dependency = Some(exclusive_dependency);
                self.weight = Some(weight);
                None
            }
            Frame::ResetStream { error, .. } => {
                warn!("Reset stream: {:?}", error);
                self.state = StreamState::Closed;
                None
            }
            Frame::PushPromise {
                promised_stream, ..
            } => {
                // TODO: handle pushes
                Frame::ResetStream {
                    stream: NonZeroStreamId::new(promised_stream)
                        .ok_or(FrameDecodeError::ZeroStreamId)?,
                    error: ErrorType::RefusedStream,
                }
                .into_bytes(send_buffer);
                None
            }
            Frame::WindowUpdate { increment, .. } => {
                self.window_remaining += self
                    .window_remaining
                    .saturating_add(u64::from(increment.get()));
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
            Frame::Settings { .. } | Frame::Ping { .. } | Frame::GoAway { .. } => {
                unreachable!("can't be sent to a stream");
            }
        })
    }

    fn decode_headers(&mut self, header_decoder: &mut hpack::Decoder) {
        header_decoder
            .decode_with_cb(&self.headers_buffer, |key, value| {
                self.response_headers.push((
                    String::from_utf8_lossy(&key).to_string(),
                    String::from_utf8_lossy(&value).to_string(),
                ));
            })
            .expect("decode_with_cb");
        self.headers_buffer.clear();
    }

    fn decode_response(&mut self) -> Response {
        Response {
            request_headers: self.request_headers.clone(),
            headers: self.response_headers.clone(),
            body: self.body_buffer.clone().freeze(),
        }
    }
}

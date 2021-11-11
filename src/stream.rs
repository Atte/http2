use crate::{connection::*, flags::*, frame::*, types::*};
use anyhow::anyhow;
use bytes::BytesMut;
use derivative::Derivative;
use log::{trace, warn};
use std::num::NonZeroU32;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
enum StreamState {
    Idle,
    ReservedLocal,
    ReservedRemote,
    Open,
    HalfClosedLocal,
    HalfClosedRemote,
    Closed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
enum Continuing {
    Headers,
    PushPromise,
}

#[derive(Derivative)]
#[derivative(Debug)]
pub struct Stream {
    pub id: NonZeroStreamId,
    pub request_id: usize,
    window_remaining: u64,
    state: StreamState,
    continuing: Option<Continuing>,
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
            request_id: 0,
            window_remaining,
            state: StreamState::Idle,
            continuing: None,
            dependency: None,
            exclusive_dependency: None,
            weight: None,
            headers_buffer: BytesMut::with_capacity(16_384 * 2),
            body_buffer: BytesMut::with_capacity(16_384 * 2),
            response_headers: Vec::new(),
        }
    }

    /// https://httpwg.org/specs/rfc7540.html#StreamStates
    pub fn transition_state(
        &mut self,
        recv: bool,
        ty: FrameType,
        flags: Flags,
    ) -> anyhow::Result<()> {
        let send = !recv;
        let original_state = self.state;

        if matches!(ty, FrameType::ResetStream) {
            if self.state == StreamState::Idle {
                return Err(anyhow!("ResetStream on Idle"));
            }
            self.state = StreamState::Closed;
        } else {
            let h = match flags {
                Flags::Headers(flags) => flags.contains(HeadersFlags::END_HEADERS),
                Flags::Continuation(flags) => {
                    matches!(self.continuing, Some(Continuing::Headers))
                        && flags.contains(ContinuationFlags::END_HEADERS)
                }
                _ => false,
            };
            let pp = match flags {
                Flags::PushPromise(flags) => flags.contains(PushPromiseFlags::END_HEADERS),
                Flags::Continuation(flags) => {
                    matches!(self.continuing, Some(Continuing::PushPromise))
                        && flags.contains(ContinuationFlags::END_HEADERS)
                }
                _ => false,
            };
            let es = match flags {
                Flags::Data(flags) => flags.contains(DataFlags::END_STREAM),
                Flags::Headers(flags) => flags.contains(HeadersFlags::END_STREAM),
                _ => false,
            };

            if self.state == StreamState::Idle {
                if send && pp {
                    self.state = StreamState::ReservedLocal;
                } else if recv && pp {
                    self.state = StreamState::ReservedRemote;
                } else if h {
                    self.state = StreamState::Open;
                }
            }

            if self.state == StreamState::ReservedLocal && send && h {
                self.state = StreamState::HalfClosedRemote;
            }

            if self.state == StreamState::ReservedRemote && recv && h {
                self.state = StreamState::HalfClosedLocal;
            }

            if self.state == StreamState::Open && send && es {
                self.state = StreamState::HalfClosedLocal;
            }

            if self.state == StreamState::Open && recv && es {
                self.state = StreamState::HalfClosedRemote;
            }

            if self.state == StreamState::HalfClosedRemote && send && es {
                self.state = StreamState::Closed;
            }

            if self.state == StreamState::HalfClosedLocal && recv && es {
                self.state = StreamState::Closed;
            }
        }

        if self.state != original_state {
            trace!(
                "stream {} {:?} -> {:?}",
                self.id,
                original_state,
                self.state
            );
        }

        Ok(())
    }

    pub fn handle_frame(
        &mut self,
        state: &mut ConnectionState,
        payload: FramePayload,
    ) -> anyhow::Result<Option<Response>> {
        let header = state.header.as_ref().expect("no header for payload");
        self.transition_state(true, header.ty, header.flags)?;
        Ok(match (header.flags, payload) {
            (Flags::Data(flags), FramePayload::Data { data, .. }) => {
                // TODO: proper flow control
                if let Some(increment) = NonZeroU32::new(header.length as u32) {
                    FramePayload::WindowUpdate { increment }.write_into(
                        &mut state.write_buf,
                        Some(self),
                        Flags::None,
                    );
                    FramePayload::WindowUpdate { increment }.write_into(
                        &mut state.write_buf,
                        None,
                        Flags::None,
                    );
                }

                self.body_buffer.extend(data);
                if flags.contains(DataFlags::END_STREAM) {
                    Some(self.decode_response())
                } else {
                    None
                }
            }
            (
                Flags::Headers(flags),
                FramePayload::Headers {
                    dependency,
                    exclusive_dependency,
                    weight,
                    fragment,
                    ..
                },
            ) => {
                if flags.contains(HeadersFlags::PRIORITY) {
                    self.dependency = dependency;
                    self.exclusive_dependency = exclusive_dependency;
                    self.weight = weight;
                }

                self.headers_buffer.extend(fragment);
                if flags.contains(HeadersFlags::END_HEADERS) {
                    self.decode_headers(&mut state.header_decoder);
                } else {
                    self.continuing = Some(Continuing::Headers);
                }

                match (
                    flags.contains(HeadersFlags::END_HEADERS),
                    flags.contains(HeadersFlags::END_STREAM),
                ) {
                    (true, true) => {
                        self.decode_headers(&mut state.header_decoder);
                        Some(self.decode_response())
                    }
                    (true, false) => {
                        self.decode_headers(&mut state.header_decoder);
                        None
                    }
                    (false, true) => None,
                    (false, false) => None,
                }
            }
            (
                Flags::None,
                FramePayload::Priority {
                    dependency,
                    exclusive_dependency,
                    weight,
                    ..
                },
            ) => {
                self.dependency = Some(dependency);
                self.exclusive_dependency = Some(exclusive_dependency);
                self.weight = Some(weight);
                None
            }
            (Flags::None, FramePayload::ResetStream { error, .. }) => {
                warn!("Reset stream: {:?}", error);
                None
            }
            (Flags::PushPromise(flags), FramePayload::PushPromise { fragment, .. }) => {
                self.headers_buffer.extend(fragment);
                if flags.contains(PushPromiseFlags::END_HEADERS) {
                    self.decode_headers(&mut state.header_decoder);
                } else {
                    self.continuing = Some(Continuing::PushPromise);
                }
                None
            }
            (Flags::None, FramePayload::WindowUpdate { increment, .. }) => {
                self.window_remaining += self
                    .window_remaining
                    .saturating_add(u64::from(increment.get()));
                None
            }
            (Flags::Continuation(flags), FramePayload::Continuation { fragment, .. }) => {
                self.headers_buffer.extend(fragment);
                if flags.contains(ContinuationFlags::END_HEADERS) {
                    self.continuing = None;

                    self.decode_headers(&mut state.header_decoder);
                    if self.state != StreamState::Open {
                        Some(self.decode_response())
                    } else {
                        None
                    }
                } else {
                    None
                }
            }
            (
                _,
                FramePayload::Settings { .. }
                | FramePayload::Ping { .. }
                | FramePayload::GoAway { .. },
            ) => {
                unreachable!("can't be sent to a stream");
            }
            _ => unreachable!("impossible Flags/FramePayload combo"),
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
            request_id: self.request_id,
            headers: self.response_headers.clone(),
            body: self.body_buffer.clone().freeze(),
        }
    }
}

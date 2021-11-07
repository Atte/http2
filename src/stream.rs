use crate::{frame::Frame, types::*};

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
    id: NonZeroStreamId,
    window_remaining: u64,
    state: StreamState,
    dependency: StreamId,
    weight: u8,
}

impl Stream {
    pub fn new(id: NonZeroStreamId, window_remaining: u64) -> Self {
        Self {
            id,
            window_remaining,
            state: StreamState::Idle,
            dependency: 0,
            weight: 0,
        }
    }

    pub fn on_frame(&mut self, frame: Frame) -> anyhow::Result<Vec<Frame>> {
        match frame {
            Frame::Data { .. } => {
                // TODO: proper flow control
                return Ok(vec![Frame::WindowUpdate {
                    stream: self.id.get(),
                    increment: MAX_WINDOW_INCREMENT,
                }]);
            }
            Frame::Headers {
                flags,
                dependency,
                weight,
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
            }
            Frame::Priority {
                dependency, weight, ..
            } => {
                self.dependency = dependency;
                self.weight = weight;
            }
            Frame::ResetStream { error, .. } => {
                eprintln!("Reset stream: {:?}", error);
                self.state = StreamState::Closed;
            }
            Frame::Settings { .. } => unreachable!("can't be sent to a stream"),
            Frame::PushPromise {
                promised_stream, ..
            } => {
                // TODO: handle pushes
                return Ok(vec![Frame::ResetStream {
                    stream: NonZeroStreamId::new(promised_stream)
                        .ok_or(FrameDecodeError::ZeroStreamId)?,
                    error: ErrorType::RefusedStream,
                }]);
            }
            Frame::Ping { .. } => unreachable!("can't be sent to a stream"),
            Frame::GoAway { .. } => unreachable!("can't be sent to a stream"),
            Frame::WindowUpdate { increment, .. } => {
                self.window_remaining +=
                    self.window_remaining.saturating_add(increment.get() as u64);
            }
            Frame::Continuation { .. } => {}
        }
        Ok(Vec::new())
    }
}

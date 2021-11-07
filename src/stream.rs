use crate::{frame::Frame, types::StreamId};

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
    dependency: StreamId,
    weight: u8,
}

impl Stream {
    pub fn on_frame(&mut self, frame: Frame) {}
}

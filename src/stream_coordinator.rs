use crate::{stream::Stream, types::*};
use derivative::Derivative;
use std::{
    collections::HashMap,
    sync::atomic::{AtomicU32, Ordering},
};

#[derive(Derivative)]
#[derivative(Debug)]
pub struct StreamCoordinator {
    client_id: AtomicU32,
    #[derivative(Debug = "ignore")]
    streams: HashMap<NonZeroStreamId, Stream>,
}

impl StreamCoordinator {
    pub fn get_mut(&mut self, id: NonZeroStreamId) -> &mut Stream {
        // TODO: initial window size
        self.streams
            .entry(id)
            .or_insert_with(|| Stream::new(id, 65_535))
    }

    /// returns None if the connection is out of stream IDs
    pub fn create_mut(&mut self) -> Option<&mut Stream> {
        NonZeroStreamId::new(self.client_id.fetch_add(2, Ordering::SeqCst))
            .map(|id| self.get_mut(id))
    }
}

impl Default for StreamCoordinator {
    #[must_use]
    fn default() -> Self {
        Self {
            client_id: AtomicU32::new(3),
            streams: HashMap::new(),
        }
    }
}

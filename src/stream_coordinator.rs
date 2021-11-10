use crate::{stream::Stream, types::*};
use dashmap::DashMap;
use std::sync::atomic::{AtomicU32, Ordering};

pub struct StreamCoordinator {
    client_id: AtomicU32,
    streams: DashMap<NonZeroStreamId, Stream>,
}

impl StreamCoordinator {
    pub fn with_stream<T, F>(&self, id: NonZeroStreamId, f: F) -> T
    where
        F: FnOnce(&mut Stream) -> T,
    {
        // TODO: initial window size
        let mut stream = self
            .streams
            .entry(id)
            .or_insert_with(|| Stream::new(id, 65_535));
        f(stream.value_mut())
    }

    pub fn with_new_stream<T, F>(&self, f: F) -> T
    where
        F: FnOnce(&mut Stream) -> T,
    {
        let id = NonZeroStreamId::new(self.client_id.fetch_add(2, Ordering::SeqCst))
            .expect("stream ID wrapped");
        self.with_stream(id, f)
    }
}

impl Default for StreamCoordinator {
    fn default() -> Self {
        Self {
            client_id: AtomicU32::new(3),
            streams: DashMap::new(),
        }
    }
}

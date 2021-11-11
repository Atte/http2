use bitflags::bitflags;

bitflags! {
    /// https://httpwg.org/specs/rfc7540.html#DATA
    #[repr(transparent)]
    pub struct DataFlags: u8 {
        /// When set, bit 0 indicates that this frame is the last that the endpoint will send for the identified stream. Setting this flag causes the stream to enter one of the "half-closed" states or the "closed" state (Section 5.1).
        const END_STREAM = 0x1;
        /// When set, bit 3 indicates that the Pad Length field and any padding that it describes are present.
        const PADDED = 0x8;
    }

    /// https://httpwg.org/specs/rfc7540.html#HEADERS
    #[repr(transparent)]
    pub struct HeadersFlags: u8 {
        /// When set, bit 0 indicates that the header block (Section 4.3) is the last that the endpoint will send for the identified stream.
        /// A HEADERS frame carries the END_STREAM flag that signals the end of a stream. However, a HEADERS frame with the END_STREAM flag set can be followed by CONTINUATION frames on the same stream. Logically, the CONTINUATION frames are part of the HEADERS frame.
        const END_STREAM = 0x1;
        /// When set, bit 2 indicates that this frame contains an entire header block (Section 4.3) and is not followed by any CONTINUATION frames.
        /// A HEADERS frame without the END_HEADERS flag set MUST be followed by a CONTINUATION frame for the same stream. A receiver MUST treat the receipt of any other type of frame or a frame on a different stream as a connection error (Section 5.4.1) of type PROTOCOL_ERROR.
        const END_HEADERS = 0x4;
        /// When set, bit 3 indicates that the Pad Length field and any padding that it describes are present.
        const PADDED = 0x8;
        /// When set, bit 5 indicates that the Exclusive Flag (E), Stream Dependency, and Weight fields are present; see Section 5.3.
        const PRIORITY = 0x20;
    }

    /// https://httpwg.org/specs/rfc7540.html#SETTINGS
    #[repr(transparent)]
    pub struct SettingsFlags: u8 {
        /// When set, bit 0 indicates that this frame acknowledges receipt and application of the peer's SETTINGS frame. When this bit is set, the payload of the SETTINGS frame MUST be empty. Receipt of a SETTINGS frame with the ACK flag set and a length field value other than 0 MUST be treated as a connection error (Section 5.4.1) of type FRAME_SIZE_ERROR. For more information, see Section 6.5.3 ("Settings Synchronization").
        const ACK = 0x1;
    }

    /// https://httpwg.org/specs/rfc7540.html#PUSH_PROMISE
    #[repr(transparent)]
    pub struct PushPromiseFlags: u8 {
        /// When set, bit 2 indicates that this frame contains an entire header block (Section 4.3) and is not followed by any CONTINUATION frames.
        /// A PUSH_PROMISE frame without the END_HEADERS flag set MUST be followed by a CONTINUATION frame for the same stream. A receiver MUST treat the receipt of any other type of frame or a frame on a different stream as a connection error (Section 5.4.1) of type PROTOCOL_ERROR.
        const END_HEADERS = 0x4;
        /// When set, bit 3 indicates that the Pad Length field and any padding that it describes are present.
        const PADDED = 0x8;
    }

    /// https://httpwg.org/specs/rfc7540.html#PING
    #[repr(transparent)]
    pub struct PingFlags: u8 {
        /// When set, bit 0 indicates that this PING frame is a PING response. An endpoint MUST set this flag in PING responses. An endpoint MUST NOT respond to PING frames containing this flag.
        const ACK = 0x1;
    }

    /// https://httpwg.org/specs/rfc7540.html#CONTINUATION
    #[repr(transparent)]
    pub struct ContinuationFlags: u8 {
        /// When set, bit 2 indicates that this frame ends a header block (Section 4.3).
        /// If the END_HEADERS bit is not set, this frame MUST be followed by another CONTINUATION frame. A receiver MUST treat the receipt of any other type of frame or a frame on a different stream as a connection error (Section 5.4.1) of type PROTOCOL_ERROR.
        const END_HEADERS = 0x4;
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, derive_more::From, derive_more::TryInto)]
pub enum Flags {
    Data(DataFlags),
    Headers(HeadersFlags),
    Settings(SettingsFlags),
    PushPromise(PushPromiseFlags),
    Ping(PingFlags),
    Continuation(ContinuationFlags),
    None,
}

use num_derive::{FromPrimitive, ToPrimitive};
use std::{collections::HashMap, num::NonZeroU32};

// Safety: value is a const, that can't be zero
pub const U31_MAX: NonZeroU32 = unsafe { NonZeroU32::new_unchecked(u32::MAX >> 1) };

pub type StreamId = u32;
pub type NonZeroStreamId = std::num::NonZeroU32;

pub type Headers = HashMap<String, Vec<String>>;

#[derive(thiserror::Error, Debug)]
pub enum DecodeError {
    #[error("Not enough bytes to decode frame")]
    TooShort,
    #[error("Unknown frame type")]
    UnknownType,
    #[error("Unexpected 0 stream ID")]
    ZeroStreamId,
    #[error("Unexpected 0 window increment")]
    ZeroWindowIncrement,
    #[error("Unknown error type")]
    UnknownErrorType,
    #[error("Invalid header: {0:?}")]
    InvalidHeader(hpack::decoder::DecoderError),
}

#[derive(thiserror::Error, Debug)]
pub enum RequestError {
    #[error("The connection ran out of stream IDs")]
    OutOfStreamIds,
    #[error("Request authority cannot be a base")]
    AuthorityCannotBeBase,
}

/// https://httpwg.org/specs/rfc7540.html#FrameTypes
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, FromPrimitive, ToPrimitive)]
#[repr(u8)]
//#[non_exhaustive]
pub enum FrameType {
    Data = 0x0,
    Headers = 0x1,
    Priority = 0x2,
    ResetStream = 0x3,
    Settings = 0x4,
    PushPromise = 0x5,
    Ping = 0x6,
    GoAway = 0x7,
    WindowUpdate = 0x8,
    Continuation = 0x9,
}

/// https://httpwg.org/specs/rfc7540.html#ErrorCodes
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, FromPrimitive, ToPrimitive)]
#[repr(u32)]
#[non_exhaustive]
pub enum ErrorType {
    /// The associated condition is not a result of an error. For example, a GOAWAY might include this code to indicate graceful shutdown of a connection.
    NoError = 0x0,
    /// The endpoint detected an unspecific protocol error. This error is for use when a more specific error code is not available.
    ProtocolError = 0x1,
    /// The endpoint encountered an unexpected internal error.
    InternalError = 0x2,
    /// The endpoint detected that its peer violated the flow-control protocol.
    FlowControlError = 0x3,
    /// The endpoint sent a SETTINGS frame but did not receive a response in a timely manner. See Section 6.5.3 ("Settings Synchronization").
    SettingsTimeout = 0x4,
    /// The endpoint received a frame after a stream was half-closed.
    StreamClosed = 0x5,
    /// The endpoint received a frame with an invalid size.
    FrameSizeError = 0x6,
    /// The endpoint refused the stream prior to performing any application processing (see Section 8.1.4 for details).
    RefusedStream = 0x7,
    /// Used by the endpoint to indicate that the stream is no longer needed.
    Cancel = 0x8,
    /// The endpoint is unable to maintain the header compression context for the connection.
    CompressionError = 0x9,
    /// The connection established in response to a CONNECT request (Section 8.3) was reset or abnormally closed.
    ConnectError = 0xa,
    /// The endpoint detected that its peer is exhibiting a behavior that might be generating excessive load.
    EnhanceYourCalm = 0xb,
    /// The underlying transport has properties that do not meet minimum security requirements (see Section 9.2).
    InadequateSecurity = 0xc,
    /// The endpoint requires that HTTP/1.1 be used instead of HTTP/2.
    Http11Required = 0xd,
}

/// https://httpwg.org/specs/rfc7540.html#SettingValues
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    FromPrimitive,
    ToPrimitive,
    enum_map::Enum,
)]
#[repr(u16)]
#[non_exhaustive]
pub enum SettingsParameter {
    /// Allows the sender to inform the remote endpoint of the maximum size of the header compression table used to decode header blocks, in octets. The encoder can select any size equal to or less than this value by using signaling specific to the header compression format inside a header block (see [COMPRESSION]). The initial value is 4,096 octets.
    HeaderTableSize = 0x1,
    /// This setting can be used to disable server push (Section 8.2). An endpoint MUST NOT send a PUSH_PROMISE frame if it receives this parameter set to a value of 0. An endpoint that has both set this parameter to 0 and had it acknowledged MUST treat the receipt of a PUSH_PROMISE frame as a connection error (Section 5.4.1) of type PROTOCOL_ERROR.
    /// The initial value is 1, which indicates that server push is permitted. Any value other than 0 or 1 MUST be treated as a connection error (Section 5.4.1) of type PROTOCOL_ERROR.
    EnablePush = 0x2,
    /// Indicates the maximum number of concurrent streams that the sender will allow. This limit is directional: it applies to the number of streams that the sender permits the receiver to create. Initially, there is no limit to this value. It is recommended that this value be no smaller than 100, so as to not unnecessarily limit parallelism.
    /// A value of 0 for SETTINGS_MAX_CONCURRENT_STREAMS SHOULD NOT be treated as special by endpoints. A zero value does prevent the creation of new streams; however, this can also happen for any limit that is exhausted with active streams. Servers SHOULD only set a zero value for short durations; if a server does not wish to accept requests, closing the connection is more appropriate.
    MaxConcurrentStreams = 0x3,
    /// Indicates the sender's initial window size (in octets) for stream-level flow control. The initial value is 2^16-1 (65,535) octets.
    /// This setting affects the window size of all streams (see Section 6.9.2).
    /// Values above the maximum flow-control window size of 2^31-1 MUST be treated as a connection error (Section 5.4.1) of type FLOW_CONTROL_ERROR.
    InitialWindowSize = 0x4,
    /// Indicates the size of the largest frame payload that the sender is willing to receive, in octets.
    /// The initial value is 2^14 (16,384) octets. The value advertised by an endpoint MUST be between this initial value and the maximum allowed frame size (2^24-1 or 16,777,215 octets), inclusive. Values outside this range MUST be treated as a connection error (Section 5.4.1) of type PROTOCOL_ERROR.
    MaxFrameSize = 0x5,
    /// This advisory setting informs a peer of the maximum size of header list that the sender is prepared to accept, in octets. The value is based on the uncompressed size of header fields, including the length of the name and value in octets plus an overhead of 32 octets for each header field.
    /// For any given request, a lower limit than what is advertised MAY be enforced. The initial value of this setting is unlimited.
    MaxHeaderListSize = 0x6,
}

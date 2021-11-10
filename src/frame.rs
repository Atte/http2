use crate::types::*;
use bytes::{Buf, BufMut, Bytes};
use log::trace;
use num_traits::FromPrimitive;
use std::num::NonZeroU32;

fn remove_padding(data: &mut Bytes) -> Bytes {
    let size = u8::from_be(data.get_u8()) as usize;
    data.copy_to_bytes(data.len() - size - 1)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Frame {
    /// https://httpwg.org/specs/rfc7540.html#DATA
    Data {
        stream: NonZeroStreamId,
        flags: DataFlags,
        flow_control_size: u32,
        data: Bytes,
    },
    /// https://httpwg.org/specs/rfc7540.html#HEADERS
    Headers {
        stream: NonZeroStreamId,
        flags: HeadersFlags,
        dependency: Option<StreamId>,
        exclusive_dependency: Option<bool>,
        weight: Option<u8>,
        fragment: Bytes,
    },
    /// https://httpwg.org/specs/rfc7540.html#PRIORITY
    Priority {
        stream: NonZeroStreamId,
        dependency: StreamId,
        exclusive_dependency: bool,
        weight: u8,
    },
    /// https://httpwg.org/specs/rfc7540.html#RST_STREAM
    ResetStream {
        stream: NonZeroStreamId,
        error: ErrorType,
    },
    /// https://httpwg.org/specs/rfc7540.html#SETTINGS
    Settings {
        flags: SettingsFlags,
        params: Vec<(SettingsParameter, u32)>,
    },
    /// https://httpwg.org/specs/rfc7540.html#PUSH_PROMISE
    PushPromise {
        stream: NonZeroStreamId,
        flags: PushPromiseFlags,
        promised_stream: u32,
        fragment: Bytes,
    },
    /// https://httpwg.org/specs/rfc7540.html#PING
    Ping { flags: PingFlags, data: Bytes },
    /// https://httpwg.org/specs/rfc7540.html#GOAWAY
    GoAway {
        last_stream: StreamId,
        error: ErrorType,
        debug: Bytes,
    },
    /// https://httpwg.org/specs/rfc7540.html#WINDOW_UPDATE
    WindowUpdate {
        stream: StreamId,
        increment: NonZeroU32,
    },
    /// https://httpwg.org/specs/rfc7540.html#CONTINUATION
    Continuation {
        stream: NonZeroStreamId,
        flags: ContinuationFlags,
        fragment: Bytes,
    },
}

impl Frame {
    pub fn try_header_from_bytes(buffer: &mut impl Buf) -> Option<Bytes> {
        if buffer.remaining() >= 9 {
            Some(buffer.copy_to_bytes(9))
        } else {
            None
        }
    }

    pub fn try_read_with_header(
        header: &Bytes,
        buffer: &mut impl Buf,
    ) -> anyhow::Result<Option<Self>> {
        let length = u32::from_be_bytes(
            [&[0_u8], &header[0..=2]]
                .concat()
                .try_into()
                .map_err(|_| FrameDecodeError::PayloadTooShort)?,
        ) as usize;

        if buffer.remaining() < length {
            return Ok(None);
        }
        let mut payload = buffer.copy_to_bytes(length);

        let typ =
            FrameType::from_u8(u8::from_be(header[3])).ok_or(FrameDecodeError::UnknownType)?;
        let flags = u8::from_be(header[4]);
        let stream = u32::from_be_bytes(
            header[5..=8]
                .try_into()
                .map_err(|_| FrameDecodeError::PayloadTooShort)?,
        ) & (u32::MAX >> 1);

        let frame = match typ {
            FrameType::Data => {
                let flags = DataFlags::from_bits_truncate(flags);
                Self::Data {
                    stream: NonZeroStreamId::new(stream).ok_or(FrameDecodeError::ZeroStreamId)?,
                    flags,
                    flow_control_size: length as u32,
                    data: if flags.contains(DataFlags::PADDED) {
                        remove_padding(&mut payload)
                    } else {
                        payload
                    },
                }
            }
            FrameType::Headers => {
                let flags = HeadersFlags::from_bits_truncate(flags);
                let mut payload = if flags.contains(HeadersFlags::PADDED) {
                    remove_padding(&mut payload)
                } else {
                    payload
                };
                let stream = NonZeroStreamId::new(stream).ok_or(FrameDecodeError::ZeroStreamId)?;
                if flags.contains(HeadersFlags::PRIORITY) {
                    let dependency = payload.get_u32();
                    Self::Headers {
                        stream,
                        flags,
                        dependency: Some(dependency & (u32::MAX >> 1)),
                        exclusive_dependency: Some((dependency >> 31) != 0),
                        weight: Some(payload.get_u8()),
                        fragment: payload,
                    }
                } else {
                    Self::Headers {
                        stream,
                        flags,
                        dependency: None,
                        exclusive_dependency: None,
                        weight: None,
                        fragment: payload,
                    }
                }
            }
            FrameType::Priority => Self::Priority {
                stream: NonZeroStreamId::new(stream).ok_or(FrameDecodeError::ZeroStreamId)?,
                dependency: u32::from_be_bytes(
                    payload[0..=3]
                        .try_into()
                        .map_err(|_| FrameDecodeError::PayloadTooShort)?,
                ) & (u32::MAX >> 1),
                exclusive_dependency: payload[0] & 0b10000_u8 != 0,
                weight: u8::from_be(payload[4]),
            },
            FrameType::ResetStream => Self::ResetStream {
                stream: NonZeroStreamId::new(stream).ok_or(FrameDecodeError::ZeroStreamId)?,
                error: ErrorType::from_u32(payload.get_u32())
                    .ok_or(FrameDecodeError::UnknownErrorType)?,
            },
            FrameType::Settings => {
                let mut params = Vec::new();
                for chunk in payload.chunks(2 + 4) {
                    // spec says to ignore unknown settings
                    if let Some(param) = SettingsParameter::from_u16(u16::from_be_bytes(
                        chunk[0..=1]
                            .try_into()
                            .map_err(|_| FrameDecodeError::PayloadTooShort)?,
                    )) {
                        params.push((
                            param,
                            u32::from_be_bytes(
                                chunk[2..=5]
                                    .try_into()
                                    .map_err(|_| FrameDecodeError::PayloadTooShort)?,
                            ),
                        ));
                    }
                }
                Self::Settings {
                    flags: SettingsFlags::from_bits_truncate(flags),
                    params,
                }
            }
            FrameType::PushPromise => {
                let flags = PushPromiseFlags::from_bits_truncate(flags);
                Self::PushPromise {
                    stream: NonZeroStreamId::new(stream).ok_or(FrameDecodeError::ZeroStreamId)?,
                    flags,
                    promised_stream: payload.get_u32() & (u32::MAX >> 1),
                    fragment: if flags.contains(PushPromiseFlags::PADDED) {
                        remove_padding(&mut payload)
                    } else {
                        payload
                    },
                }
            }
            FrameType::Ping => Self::Ping {
                flags: PingFlags::from_bits_truncate(flags),
                data: payload,
            },
            FrameType::GoAway => Self::GoAway {
                last_stream: payload.get_u32() & (u32::MAX >> 1),
                error: ErrorType::from_u32(payload.get_u32())
                    .ok_or(FrameDecodeError::UnknownErrorType)?,
                debug: payload,
            },
            FrameType::WindowUpdate => Self::WindowUpdate {
                stream,
                increment: NonZeroU32::new(payload.get_u32() & (u32::MAX >> 1))
                    .ok_or(FrameDecodeError::ZeroWindowIncrement)?,
            },
            FrameType::Continuation => Self::Continuation {
                stream: NonZeroStreamId::new(stream).ok_or(FrameDecodeError::ZeroStreamId)?,
                flags: ContinuationFlags::from_bits_truncate(flags),
                fragment: payload,
            },
        };
        trace!("[RECV] {:#?}", frame);
        Ok(Some(frame))
    }

    fn typ(&self) -> FrameType {
        match self {
            Self::Data { .. } => FrameType::Data,
            Self::Headers { .. } => FrameType::Headers,
            Self::Priority { .. } => FrameType::Priority,
            Self::ResetStream { .. } => FrameType::ResetStream,
            Self::Settings { .. } => FrameType::Settings,
            Self::PushPromise { .. } => FrameType::PushPromise,
            Self::Ping { .. } => FrameType::Ping,
            Self::GoAway { .. } => FrameType::GoAway,
            Self::WindowUpdate { .. } => FrameType::WindowUpdate,
            Self::Continuation { .. } => FrameType::Continuation,
        }
    }

    fn stream(&self) -> StreamId {
        match self {
            Self::Data { stream, .. }
            | Self::Headers { stream, .. }
            | Self::Priority { stream, .. }
            | Self::ResetStream { stream, .. }
            | Self::PushPromise { stream, .. }
            | Self::Continuation { stream, .. } => stream.get(),
            Self::WindowUpdate { stream, .. } => *stream,
            Self::Settings { .. } | Self::Ping { .. } | Self::GoAway { .. } => 0,
        }
    }

    fn flags(&self) -> u8 {
        match self {
            Self::Data { flags, .. } => flags.bits(),
            Self::Headers { flags, .. } => flags.bits(),
            Self::Settings { flags, .. } => flags.bits(),
            Self::PushPromise { flags, .. } => flags.bits(),
            Self::Ping { flags, .. } => flags.bits(),
            Self::Continuation { flags, .. } => flags.bits(),
            Self::Priority { .. }
            | Self::ResetStream { .. }
            | Self::GoAway { .. }
            | Self::WindowUpdate { .. } => 0,
        }
    }

    fn into_payload(self) -> Bytes {
        match self {
            Self::Data { data, .. } | Self::Ping { data, .. } => data,
            Self::Headers {
                flags,
                dependency,
                exclusive_dependency,
                weight,
                fragment,
                ..
            } => {
                if flags.contains(HeadersFlags::PRIORITY) {
                    let mut payload: Vec<u8> = dependency
                        .expect("missing header dependency")
                        .to_be_bytes()
                        .to_vec();
                    if exclusive_dependency.expect("missing header exclusive_dependency") {
                        payload[0] &= 0b10000_u8;
                    }
                    payload.push(weight.expect("missing header weight").to_be());
                    payload.extend(fragment);
                    payload.into()
                } else {
                    fragment
                }
            }
            Self::Priority {
                dependency,
                exclusive_dependency,
                weight,
                ..
            } => {
                let mut payload: Vec<u8> = dependency.to_be_bytes().to_vec();
                if exclusive_dependency {
                    payload[0] &= 0b10000_u8;
                }
                payload.push(weight.to_be());
                payload.into()
            }
            Self::ResetStream { error, .. } => (error as u32).to_be_bytes().to_vec().into(),
            Self::Settings { params, .. } => {
                let mut payload = Vec::with_capacity((2 + 4) * params.len());
                for (key, value) in params {
                    payload.extend((key as u16).to_be_bytes());
                    payload.extend(value.to_be_bytes());
                }
                payload.into()
            }
            Self::PushPromise {
                promised_stream,
                fragment,
                ..
            } => {
                let mut payload = promised_stream.to_be_bytes().to_vec();
                payload.extend(fragment);
                payload.into()
            }
            Self::GoAway {
                last_stream,
                error,
                debug,
                ..
            } => {
                let mut payload = last_stream.to_be_bytes().to_vec();
                payload.extend((error as u32).to_be_bytes());
                payload.extend(debug);
                payload.into()
            }
            Self::WindowUpdate { increment, .. } => increment.get().to_be_bytes().to_vec().into(),
            Self::Continuation { fragment, .. } => fragment,
        }
    }

    pub fn into_bytes(self, buffer: &mut impl BufMut) {
        trace!("[SEND] {:#?}", self);

        let typ = self.typ() as u8;
        let flags = self.flags();
        let stream = self.stream();
        let payload = self.into_payload();
        let length = payload.len().to_be_bytes();

        buffer.put(&length[length.len() - 3..]);
        buffer.put_u8(typ.to_be());
        buffer.put_u8(flags.to_be());
        buffer.put(&stream.to_be_bytes()[..]);
        buffer.put(&payload[..]);
    }
}

impl From<Vec<(SettingsParameter, u32)>> for Frame {
    fn from(params: Vec<(SettingsParameter, u32)>) -> Self {
        Self::Settings {
            flags: SettingsFlags::empty(),
            params,
        }
    }
}

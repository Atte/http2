use crate::{flags::*, stream::*, types::*};
use bytes::{Buf, BufMut, Bytes, BytesMut};
use log::trace;
use num_traits::FromPrimitive;
use std::num::NonZeroU32;

fn remove_padding(data: &mut Bytes) -> Bytes {
    let size = u8::from_be(data.get_u8()) as usize;
    data.copy_to_bytes(data.len() - size - 1)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrameHeader {
    pub length: usize,
    pub ty: FrameType,
    pub flags: Flags,
    pub stream_id: StreamId,
}

impl FrameHeader {
    pub const BYTES: usize = 9;

    pub fn write_into(self, buffer: &mut impl BufMut) {
        buffer.put(&(self.length as u32).to_be_bytes()[1..]);
        buffer.put_u8((self.ty as u8).to_be());
        buffer.put_u8(
            match self.flags {
                Flags::Data(flags) => flags.bits(),
                Flags::Headers(flags) => flags.bits(),
                Flags::Settings(flags) => flags.bits(),
                Flags::PushPromise(flags) => flags.bits(),
                Flags::Ping(flags) => flags.bits(),
                Flags::Continuation(flags) => flags.bits(),
                Flags::None => 0_u8,
            }
            .to_be(),
        );
        buffer.put(&self.stream_id.to_be_bytes()[..]);
    }
}

impl TryFrom<&mut BytesMut> for FrameHeader {
    type Error = FrameDecodeError;
    fn try_from(buffer: &mut BytesMut) -> Result<FrameHeader, FrameDecodeError> {
        if buffer.remaining() >= Self::BYTES {
            let length = u32::from_be_bytes(
                [&[0_u8], buffer.copy_to_bytes(3).as_ref()]
                    .concat()
                    .try_into()
                    .unwrap(),
            ) as usize;
            let ty = FrameType::from_u8(buffer.get_u8()).ok_or(FrameDecodeError::UnknownType)?;
            let flags = buffer.get_u8();
            let stream_id =
                u32::from_be_bytes(buffer.copy_to_bytes(4).as_ref().try_into().unwrap())
                    & (u32::MAX >> 1);
            let header = Self {
                length,
                ty,
                flags: match ty {
                    FrameType::Data => DataFlags::from_bits_truncate(flags).into(),
                    FrameType::Headers => HeadersFlags::from_bits_truncate(flags).into(),
                    FrameType::Settings => SettingsFlags::from_bits_truncate(flags).into(),
                    FrameType::PushPromise => PushPromiseFlags::from_bits_truncate(flags).into(),
                    FrameType::Ping => PingFlags::from_bits_truncate(flags).into(),
                    FrameType::Continuation => ContinuationFlags::from_bits_truncate(flags).into(),
                    _ => Flags::None,
                },
                stream_id,
            };
            trace!("[RECV] {:#?}", header);
            Ok(header)
        } else {
            Err(FrameDecodeError::TooShort)
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FramePayload {
    /// https://httpwg.org/specs/rfc7540.html#DATA
    Data { data: Bytes },
    /// https://httpwg.org/specs/rfc7540.html#HEADERS
    Headers {
        dependency: Option<StreamId>,
        exclusive_dependency: Option<bool>,
        weight: Option<u8>,
        fragment: Bytes,
    },
    /// https://httpwg.org/specs/rfc7540.html#PRIORITY
    Priority {
        dependency: StreamId,
        exclusive_dependency: bool,
        weight: u8,
    },
    /// https://httpwg.org/specs/rfc7540.html#RST_STREAM
    ResetStream { error: ErrorType },
    /// https://httpwg.org/specs/rfc7540.html#SETTINGS
    Settings {
        params: Vec<(SettingsParameter, u32)>,
    },
    /// https://httpwg.org/specs/rfc7540.html#PUSH_PROMISE
    PushPromise {
        promised_stream: NonZeroStreamId,
        fragment: Bytes,
    },
    /// https://httpwg.org/specs/rfc7540.html#PING
    Ping { data: Bytes },
    /// https://httpwg.org/specs/rfc7540.html#GOAWAY
    GoAway {
        last_stream: StreamId,
        error: ErrorType,
        debug: Bytes,
    },
    /// https://httpwg.org/specs/rfc7540.html#WINDOW_UPDATE
    WindowUpdate { increment: NonZeroU32 },
    /// https://httpwg.org/specs/rfc7540.html#CONTINUATION
    Continuation { fragment: Bytes },
}

impl FramePayload {
    pub fn try_from(buffer: &mut impl Buf, header: &FrameHeader) -> Result<Self, FrameDecodeError> {
        if buffer.remaining() < header.length {
            return Err(FrameDecodeError::TooShort);
        }
        let mut payload = buffer.copy_to_bytes(header.length);

        let frame = match (header.ty, header.flags) {
            (FrameType::Data, Flags::Data(flags)) => Self::Data {
                data: if flags.contains(DataFlags::PADDED) {
                    remove_padding(&mut payload)
                } else {
                    payload
                },
            },
            (FrameType::Headers, Flags::Headers(flags)) => {
                let mut payload = if flags.contains(HeadersFlags::PADDED) {
                    remove_padding(&mut payload)
                } else {
                    payload
                };
                if flags.contains(HeadersFlags::PRIORITY) {
                    let dependency = payload.get_u32();
                    Self::Headers {
                        dependency: Some(dependency & (u32::MAX >> 1)),
                        exclusive_dependency: Some((dependency >> 31) != 0),
                        weight: Some(payload.get_u8()),
                        fragment: payload,
                    }
                } else {
                    Self::Headers {
                        dependency: None,
                        exclusive_dependency: None,
                        weight: None,
                        fragment: payload,
                    }
                }
            }
            (FrameType::Priority, Flags::None) => {
                let dependency = payload.get_u32();
                Self::Priority {
                    dependency: dependency & (u32::MAX >> 1),
                    exclusive_dependency: dependency & (1 << 31) != 0,
                    weight: payload.get_u8(),
                }
            }
            (FrameType::ResetStream, Flags::None) => Self::ResetStream {
                error: ErrorType::from_u32(payload.get_u32())
                    .ok_or(FrameDecodeError::UnknownErrorType)?,
            },
            (FrameType::Settings, Flags::Settings(_)) => {
                let mut params = Vec::new();
                while payload.has_remaining() {
                    let param = payload.get_u16();
                    let value = payload.get_u32();
                    // spec says to ignore unknown settings
                    if let Some(param) = SettingsParameter::from_u16(param) {
                        params.push((param, value));
                    }
                }
                Self::Settings { params }
            }
            (FrameType::PushPromise, Flags::PushPromise(flags)) => Self::PushPromise {
                promised_stream: NonZeroStreamId::new(payload.get_u32() & (u32::MAX >> 1))
                    .ok_or(FrameDecodeError::ZeroStreamId)?,
                fragment: if flags.contains(PushPromiseFlags::PADDED) {
                    remove_padding(&mut payload)
                } else {
                    payload
                },
            },
            (FrameType::Ping, Flags::Ping(_)) => Self::Ping { data: payload },
            (FrameType::GoAway, Flags::None) => Self::GoAway {
                last_stream: payload.get_u32() & (u32::MAX >> 1),
                error: ErrorType::from_u32(payload.get_u32())
                    .ok_or(FrameDecodeError::UnknownErrorType)?,
                debug: payload,
            },
            (FrameType::WindowUpdate, Flags::None) => Self::WindowUpdate {
                increment: NonZeroU32::new(payload.get_u32() & (u32::MAX >> 1))
                    .ok_or(FrameDecodeError::ZeroWindowIncrement)?,
            },
            (FrameType::Continuation, Flags::Continuation(_)) => {
                Self::Continuation { fragment: payload }
            }
            _ => unreachable!("impossible FrameType/Flags combos"),
        };
        //trace!("[RECV] {:#?}", frame);
        Ok(frame)
    }

    fn into_payload(self) -> Bytes {
        match self {
            Self::Data { data, .. } | Self::Ping { data, .. } => data,
            Self::Headers {
                dependency,
                exclusive_dependency,
                weight,
                fragment,
                ..
            } => match (dependency, exclusive_dependency, weight) {
                (Some(dependency), Some(exclusive_dependency), Some(weight)) => {
                    let mut payload: Vec<u8> = dependency.to_be_bytes().to_vec();
                    if exclusive_dependency {
                        payload[0] &= 0b10000_u8;
                    }
                    payload.push(weight.to_be());
                    payload.extend(fragment);
                    payload.into()
                }
                _ => fragment,
            },
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
                let mut payload = promised_stream.get().to_be_bytes().to_vec();
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

    pub fn write_into(
        self,
        buffer: &mut impl BufMut,
        stream: Option<&mut Stream>,
        flags: impl Into<Flags>,
    ) {
        let ty: FrameType = (&self).into();
        let payload = self.into_payload();
        let header = FrameHeader {
            length: payload.len(),
            ty,
            flags: flags.into(),
            stream_id: stream.map_or(0, |s| s.id.get()),
        };

        trace!("[SEND] {:#?}", header);
        header.write_into(buffer);

        //trace!("[SEND] {:#?}", payload);
        buffer.put(&payload[..]);
    }
}

impl From<Vec<(SettingsParameter, u32)>> for FramePayload {
    fn from(params: Vec<(SettingsParameter, u32)>) -> Self {
        Self::Settings { params }
    }
}

impl From<&FramePayload> for FrameType {
    fn from(payload: &FramePayload) -> Self {
        match payload {
            FramePayload::Data { .. } => Self::Data,
            FramePayload::Headers { .. } => Self::Headers,
            FramePayload::Priority { .. } => Self::Priority,
            FramePayload::ResetStream { .. } => Self::ResetStream,
            FramePayload::Settings { .. } => Self::Settings,
            FramePayload::PushPromise { .. } => Self::PushPromise,
            FramePayload::Ping { .. } => Self::Ping,
            FramePayload::GoAway { .. } => Self::GoAway,
            FramePayload::WindowUpdate { .. } => Self::WindowUpdate,
            FramePayload::Continuation { .. } => Self::Continuation,
        }
    }
}

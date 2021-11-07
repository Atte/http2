use crate::types::*;
use async_std::io::{self, ReadExt, WriteExt};
use num_traits::FromPrimitive;
use std::collections::HashMap;

fn remove_padding(data: Vec<u8>) -> Vec<u8> {
    let size = u8::from_be(data[0]) as usize;
    data[1..(data.len() - size)].to_vec()
}

#[derive(thiserror::Error, Debug)]
pub enum FrameDecodeError {
    #[error("Unknown frame type")]
    UnknownType,
    #[error("Unexpected 0 stream ID")]
    ZeroStreamId,
    #[error("Payload is shorter than expexted")]
    PayloadTooShort,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Frame {
    /// https://httpwg.org/specs/rfc7540.html#DATA
    Data {
        stream: NonZeroStreamId,
        flags: DataFlags,
        data: Vec<u8>,
    },
    /// https://httpwg.org/specs/rfc7540.html#HEADERS
    Headers {
        stream: NonZeroStreamId,
        flags: HeadersFlags,
        dependency: StreamId,
        exclusive_dependency: bool,
        weight: u8,
        fragment: Vec<u8>,
    },
    /// https://httpwg.org/specs/rfc7540.html#PRIORITY
    Priority {
        stream: NonZeroStreamId,
        dependency: StreamId,
        exclusive_dependency: bool,
        weight: u8,
    },
    /// https://httpwg.org/specs/rfc7540.html#RST_STREAM
    ResetStream { stream: NonZeroStreamId, error: u32 },
    /// https://httpwg.org/specs/rfc7540.html#SETTINGS
    Settings {
        flags: SettingsFlags,
        params: HashMap<SettingsParameter, u32>,
    },
    /// https://httpwg.org/specs/rfc7540.html#PUSH_PROMISE
    PushPromise {
        stream: NonZeroStreamId,
        flags: PushPromiseFlags,
        promised_stream: u32,
        fragment: Vec<u8>,
    },
    /// https://httpwg.org/specs/rfc7540.html#PING
    Ping { flags: PingFlags, data: Vec<u8> },
    /// https://httpwg.org/specs/rfc7540.html#GOAWAY
    GoAway {
        last_stream: StreamId,
        error: u32,
        debug: Vec<u8>,
    },
    /// https://httpwg.org/specs/rfc7540.html#WINDOW_UPDATE
    WindowUpdate {
        stream: StreamId,
        size_increment: u32,
    },
    /// https://httpwg.org/specs/rfc7540.html#CONTINUATION
    Continuation {
        stream: NonZeroStreamId,
        flags: ContinuationFlags,
        fragment: Vec<u8>,
    },
}

impl Frame {
    pub fn new_settings_ack() -> Self {
        Self::Settings {
            flags: SettingsFlags::ACK,
            params: HashMap::new(),
        }
    }

    pub async fn read_from(readable: &mut (impl io::Read + Unpin)) -> anyhow::Result<Self> {
        let mut header = [0u8; 9];
        readable.read_exact(&mut header).await?;

        let length = u32::from_be_bytes(
            [&[0u8], &header[0..=2]]
                .concat()
                .try_into()
                .map_err(|_| FrameDecodeError::PayloadTooShort)?,
        );

        // TODO: use MaybeUninit for payload buffer
        let mut payload = vec![0u8; length as usize];
        readable.read_exact(&mut payload).await?;

        let typ =
            FrameType::from_u8(u8::from_be(header[3])).ok_or(FrameDecodeError::UnknownType)?;
        let flags = u8::from_be(header[4]);
        let stream = u32::from_be_bytes(
            header[5..=8]
                .try_into()
                .map_err(|_| FrameDecodeError::PayloadTooShort)?,
        ) & (u32::MAX >> 1);

        Ok(match typ {
            FrameType::Data => {
                let flags = DataFlags::from_bits_truncate(flags);
                Self::Data {
                    stream: NonZeroStreamId::new(stream).ok_or(FrameDecodeError::ZeroStreamId)?,
                    flags,
                    data: if flags.contains(DataFlags::PADDED) {
                        remove_padding(payload)
                    } else {
                        payload
                    },
                }
            }
            FrameType::Headers => {
                let flags = HeadersFlags::from_bits_truncate(flags);
                let payload = if flags.contains(HeadersFlags::PADDED) {
                    remove_padding(payload)
                } else {
                    payload
                };
                let stream = NonZeroStreamId::new(stream).ok_or(FrameDecodeError::ZeroStreamId)?;
                if flags.contains(HeadersFlags::PRIORITY) {
                    Self::Headers {
                        stream,
                        flags,
                        dependency: u32::from_be_bytes(
                            payload[0..=3]
                                .try_into()
                                .map_err(|_| FrameDecodeError::PayloadTooShort)?,
                        ) & (u32::MAX >> 1),
                        exclusive_dependency: payload[0] & 0b10000u8 != 0,
                        weight: u8::from_be(payload[4]),
                        fragment: payload[5..].to_vec(),
                    }
                } else {
                    Self::Headers {
                        stream,
                        flags,
                        dependency: 0,
                        exclusive_dependency: false,
                        weight: 0,
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
                exclusive_dependency: payload[0] & 0b10000u8 != 0,
                weight: u8::from_be(payload[4]),
            },
            FrameType::ResetStream => Self::ResetStream {
                stream: NonZeroStreamId::new(stream).ok_or(FrameDecodeError::ZeroStreamId)?,
                error: u32::from_be_bytes(
                    payload[0..=3]
                        .try_into()
                        .map_err(|_| FrameDecodeError::PayloadTooShort)?,
                ),
            },
            FrameType::Settings => {
                let mut params = HashMap::with_capacity(payload.len() / (2 + 4));
                for chunk in payload.chunks(2 + 4) {
                    // spec says to ignore unknown settings
                    if let Some(param) = SettingsParameter::from_u16(u16::from_be_bytes(
                        chunk[0..=1]
                            .try_into()
                            .map_err(|_| FrameDecodeError::PayloadTooShort)?,
                    )) {
                        params.insert(
                            param,
                            u32::from_be_bytes(
                                chunk[2..=5]
                                    .try_into()
                                    .map_err(|_| FrameDecodeError::PayloadTooShort)?,
                            ),
                        );
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
                    promised_stream: u32::from_be_bytes(
                        payload[0..=3]
                            .try_into()
                            .map_err(|_| FrameDecodeError::PayloadTooShort)?,
                    ) & (u32::MAX >> 1),
                    fragment: if flags.contains(PushPromiseFlags::PADDED) {
                        remove_padding(payload)
                    } else {
                        payload
                    }[4..]
                        .to_vec(),
                }
            }
            FrameType::Ping => Self::Ping {
                flags: PingFlags::from_bits_truncate(flags),
                data: payload,
            },
            FrameType::GoAway => Self::GoAway {
                last_stream: u32::from_be_bytes(
                    payload[0..=3]
                        .try_into()
                        .map_err(|_| FrameDecodeError::PayloadTooShort)?,
                ) & (u32::MAX >> 1),
                error: u32::from_be_bytes(
                    payload[4..=7]
                        .try_into()
                        .map_err(|_| FrameDecodeError::PayloadTooShort)?,
                ),
                debug: payload[8..].to_vec(),
            },
            FrameType::WindowUpdate => Self::WindowUpdate {
                stream,
                size_increment: u32::from_be_bytes(
                    payload[0..=3]
                        .try_into()
                        .map_err(|_| FrameDecodeError::PayloadTooShort)?,
                ) & (u32::MAX >> 1),
            },
            FrameType::Continuation => Self::Continuation {
                stream: NonZeroStreamId::new(stream).ok_or(FrameDecodeError::ZeroStreamId)?,
                flags: ContinuationFlags::from_bits_truncate(flags),
                fragment: payload,
            },
        })
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
            Self::Data { stream, .. } => stream.get(),
            Self::Headers { stream, .. } => stream.get(),
            Self::Priority { stream, .. } => stream.get(),
            Self::ResetStream { stream, .. } => stream.get(),
            Self::Settings { .. } => 0,
            Self::PushPromise { stream, .. } => stream.get(),
            Self::Ping { .. } => 0,
            Self::GoAway { .. } => 0,
            Self::WindowUpdate { stream, .. } => *stream,
            Self::Continuation { stream, .. } => stream.get(),
        }
    }

    fn flags(&self) -> u8 {
        match self {
            Self::Data { flags, .. } => flags.bits(),
            Self::Headers { flags, .. } => flags.bits(),
            Self::Priority { .. } => 0,
            Self::ResetStream { .. } => 0,
            Self::Settings { flags, .. } => flags.bits(),
            Self::PushPromise { flags, .. } => flags.bits(),
            Self::Ping { flags, .. } => flags.bits(),
            Self::GoAway { .. } => 0,
            Self::WindowUpdate { .. } => 0,
            Self::Continuation { flags, .. } => flags.bits(),
        }
    }

    fn into_payload(self) -> Vec<u8> {
        match self {
            Self::Data { data, .. } => data,
            Self::Headers {
                flags,
                dependency,
                exclusive_dependency,
                weight,
                fragment,
                ..
            } => {
                if flags.contains(HeadersFlags::PRIORITY) {
                    let mut payload: Vec<u8> = dependency.to_be_bytes().to_vec();
                    if exclusive_dependency {
                        payload[0] &= 0b10000u8;
                    }
                    payload.push(weight.to_be());
                    payload.extend(fragment);
                    payload
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
                    payload[0] &= 0b10000u8;
                }
                payload.push(weight.to_be());
                payload
            }
            Self::ResetStream { error, .. } => error.to_be_bytes().to_vec(),
            Self::Settings { params, .. } => {
                let mut payload = Vec::with_capacity((2 + 4) * params.len());
                for (key, value) in params.iter() {
                    payload.extend((*key as u16).to_be_bytes());
                    payload.extend(value.to_be_bytes());
                }
                payload
            }
            Self::PushPromise {
                promised_stream,
                fragment,
                ..
            } => {
                let mut payload = promised_stream.to_be_bytes().to_vec();
                payload.extend(fragment);
                payload
            }
            Self::Ping { data, .. } => data,
            Self::GoAway {
                last_stream,
                error,
                debug,
                ..
            } => {
                let mut payload = last_stream.to_be_bytes().to_vec();
                payload.extend(error.to_be_bytes());
                payload.extend(debug);
                payload
            }
            Self::WindowUpdate { size_increment, .. } => size_increment.to_be_bytes().to_vec(),
            Self::Continuation { fragment, .. } => fragment,
        }
    }

    pub async fn write_into(self, writable: &mut (impl io::Write + Unpin)) -> io::Result<()> {
        let typ = self.typ() as u8;
        let flags = self.flags();
        let stream = self.stream();
        let payload = self.into_payload();

        writable
            .write_all(&payload.len().to_be_bytes()[1..])
            .await?;
        writable.write_all(&[typ.to_be(), flags.to_be()]).await?;
        writable.write_all(&stream.to_be_bytes()).await?;
        writable.write_all(&payload).await?;
        Ok(())
    }
}

impl From<HashMap<SettingsParameter, u32>> for Frame {
    fn from(params: HashMap<SettingsParameter, u32>) -> Self {
        Self::Settings {
            flags: SettingsFlags::empty(),
            params,
        }
    }
}

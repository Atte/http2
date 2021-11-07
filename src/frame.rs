use crate::enums::*;
use async_std::io::{self, ReadExt, WriteExt};
use num_traits::{FromPrimitive, ToPrimitive};
use std::{
    collections::HashMap,
    convert::{From, TryFrom},
};

#[derive(Debug, Clone)]
pub struct Frame {
    pub typ: FrameType,
    pub flags: u8,
    pub stream_id: u32,
    pub payload: Vec<u8>,
}

impl Frame {
    pub fn new(typ: FrameType, flags: u8, stream_id: u32, payload: Vec<u8>) -> Self {
        Self {
            typ,
            flags,
            stream_id,
            payload,
        }
    }

    pub async fn try_from_stream(stream: &mut (impl io::Read + Unpin)) -> io::Result<Self> {
        let mut header = [0u8; 9];
        stream.read_exact(&mut header).await?;
        // unwrap: the length of the slice is always 4
        let length = u32::from_be_bytes([&[0u8], &header[0..=2]].concat().try_into().unwrap());
        // TODO: use MaybeUninit for payload buffer
        let mut payload = vec![0u8; length as usize];
        stream.read_exact(&mut payload).await?;
        Ok(Self {
            typ: FrameType::from_u8(u8::from_be(header[3])).ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::Other,
                    format!("unknown frame type: {:?}", u8::from_be(header[3])),
                )
            })?,
            flags: u8::from_be(header[4]),
            // unwrap: the length of the slice is always 4
            stream_id: u32::from_be_bytes(header[5..=8].try_into().unwrap()) & (u32::MAX >> 1),
            payload,
        })
    }

    pub async fn write_into(&self, stream: &mut (impl io::Write + Unpin)) -> io::Result<()> {
        stream
            .write_all(&self.payload.len().to_be_bytes()[1..])
            .await?;
        // unwrap: FrameType is repr(u8)
        stream
            .write_all(&[self.typ.to_u8().unwrap().to_be()])
            .await?;
        stream.write_all(&[self.flags.to_be()]).await?;
        stream.write_all(&self.stream_id.to_be_bytes()).await?;
        stream.write_all(&self.payload).await?;
        Ok(())
    }

    pub fn append_u8(&mut self, value: impl ToPrimitive) {
        self.payload
            .push(value.to_u8().expect("invalid value passed into u8").to_be());
    }
    pub fn append_u16(&mut self, value: impl ToPrimitive) {
        self.payload.extend(
            value
                .to_u16()
                .expect("invalid value passed into u16")
                .to_be_bytes(),
        );
    }
    pub fn append_u32(&mut self, value: impl ToPrimitive) {
        self.payload.extend(
            value
                .to_u32()
                .expect("invalid value passed into u32")
                .to_be_bytes(),
        );
    }
}

/// https://httpwg.org/specs/rfc7540.html#SETTINGS
#[derive(Debug, Clone)]
pub struct SettingsFrame {
    ack: bool,
    params: HashMap<SettingsParameter, u32>,
}

impl SettingsFrame {
    pub fn ack() -> Self {
        Self {
            ack: true,
            params: HashMap::new(),
        }
    }

    pub async fn write_into(&self, mut stream: &mut (impl io::Write + Unpin)) -> io::Result<()> {
        let mut payload = Vec::with_capacity((2 + 4) * self.params.len());
        for (key, value) in self.params.iter() {
            payload.extend((*key as u16).to_be_bytes());
            payload.extend(value.to_be_bytes());
        }
        let frame = Frame::new(
            FrameType::Settings,
            if self.ack {
                SettingsFlags::Ack as u8
            } else {
                0
            },
            0,
            payload,
        );
        frame.write_into(&mut stream).await?;
        Ok(())
    }
}

impl From<SettingsFrame> for HashMap<SettingsParameter, u32> {
    fn from(frame: SettingsFrame) -> Self {
        frame.params
    }
}

impl TryFrom<Frame> for SettingsFrame {
    type Error = &'static str;
    fn try_from(frame: Frame) -> Result<Self, Self::Error> {
        if frame.typ != FrameType::Settings {
            return Err("wrong frame type");
        }

        let mut params = HashMap::with_capacity(frame.payload.len() / (2 + 4));
        for chunk in frame.payload.chunks(2 + 4) {
            // spec says to ignore unknown settings
            if let Some(param) = SettingsParameter::from_u16(u16::from_be_bytes(
                // unwrap: the length of the slice is always 2
                chunk[0..=1].try_into().unwrap(),
            )) {
                params.insert(
                    param,
                    u32::from_be_bytes(
                        // unwrap: the length of the slice is always 4
                        chunk[2..=5].try_into().unwrap(),
                    ),
                );
            }
        }
        Ok(Self {
            ack: frame.flags & (SettingsFlags::Ack as u8) != 0,
            params,
        })
    }
}

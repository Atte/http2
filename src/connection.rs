use crate::{frame::*, stream::Stream, types::*};
use async_std::io;
use async_std::{
    io::WriteExt,
    net::{TcpStream, ToSocketAddrs},
};
use enum_map::{enum_map, EnumMap};
use std::collections::HashMap;

pub struct Connection {
    socket: TcpStream,
    streams: HashMap<NonZeroStreamId, Stream>,
    their_settings: EnumMap<SettingsParameter, u32>,
    window_remaining: u64,
}

impl Connection {
    pub async fn connect(addr: impl ToSocketAddrs) -> io::Result<Self> {
        let mut socket = TcpStream::connect(addr).await?;

        // client connection preface
        socket
            .write_all(b"PRI * HTTP/2.0\r\n\r\nSM\r\n\r\n")
            .await?;

        let frame: Frame = vec![
            (SettingsParameter::HeaderTableSize, 0),
            (SettingsParameter::EnablePush, 0),
            (
                SettingsParameter::InitialWindowSize,
                MAX_WINDOW_INCREMENT.get(),
            ),
        ]
        .into();
        frame.write_into(&mut socket).await?;

        socket.flush().await?;

        Ok(Self {
            socket,
            streams: HashMap::new(),
            their_settings: enum_map! {
                SettingsParameter::HeaderTableSize => 4096,
                SettingsParameter::EnablePush => 1,
                SettingsParameter::MaxConcurrentStreams => u32::MAX,
                SettingsParameter::InitialWindowSize => 65_535,
                SettingsParameter::MaxFrameSize => 16_384,
                SettingsParameter::MaxHeaderListSize => u32::MAX,
            },
            window_remaining: 65_535,
        })
    }

    fn stream(&mut self, stream: NonZeroStreamId) -> &mut Stream {
        self.streams
            .entry(stream)
            .or_insert_with(|| Stream::new(stream, self.window_remaining))
    }

    async fn handle_frame(&mut self) -> anyhow::Result<()> {
        let frame = Frame::read_from(&mut self.socket).await?;
        let response = match frame {
            Frame::Data { stream, .. } => {
                let mut response = self.stream(stream).on_frame(frame)?;
                // TODO: proper flow control
                response.push(Frame::WindowUpdate {
                    stream: 0,
                    increment: MAX_WINDOW_INCREMENT,
                });
                response
            }
            Frame::Headers { stream, .. } => self.stream(stream).on_frame(frame)?,
            Frame::Priority { stream, .. } => self.stream(stream).on_frame(frame)?,
            Frame::ResetStream { stream, .. } => self.stream(stream).on_frame(frame)?,
            Frame::Settings { params, .. } => {
                for (key, value) in params {
                    self.their_settings[key] = value;
                }
                vec![Frame::Settings {
                    flags: SettingsFlags::ACK,
                    params: Vec::new(),
                }]
            }
            Frame::PushPromise { stream, .. } => self.stream(stream).on_frame(frame)?,
            Frame::Ping { flags, data, .. } => {
                if !flags.contains(PingFlags::ACK) {
                    if data.len() == 8 {
                        vec![Frame::Ping {
                            flags: PingFlags::ACK,
                            data,
                        }]
                    } else {
                        vec![Frame::GoAway {
                            last_stream: self.streams.keys().max().map_or(0, |stream| stream.get()),
                            error: ErrorType::ProtocolError,
                            debug: "invalid ping payload length".as_bytes().to_vec(),
                        }]
                    }
                } else {
                    Vec::new()
                }
            }
            Frame::GoAway { error, debug, .. } => {
                eprintln!("Go away: {:?}", error);
                if let Ok(debug) = String::from_utf8(debug) {
                    eprintln!("Go away debug: {}", debug);
                }
                Vec::new()
            }
            Frame::WindowUpdate {
                stream, increment, ..
            } => {
                if let Some(stream) = NonZeroStreamId::new(stream) {
                    self.stream(stream).on_frame(frame)?
                } else {
                    self.window_remaining =
                        self.window_remaining.saturating_add(increment.get() as u64);
                    Vec::new()
                }
            }
            Frame::Continuation { stream, .. } => self.stream(stream).on_frame(frame)?,
        };
        for frame in response.into_iter() {
            frame.write_into(&mut self.socket).await?;
        }
        Ok(())
    }
}

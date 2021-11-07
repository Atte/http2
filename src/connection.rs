use crate::{
    frame::*,
    stream::Stream,
    types::{NonZeroStreamId, SettingsParameter},
};
use async_std::io;
use async_std::{
    io::WriteExt,
    net::{TcpStream, ToSocketAddrs},
};
use maplit::hashmap;
use std::collections::HashMap;

pub struct Connection {
    socket: TcpStream,
    streams: HashMap<NonZeroStreamId, Stream>,
    their_settings: HashMap<SettingsParameter, u32>,
}

impl Connection {
    pub async fn connect(addr: impl ToSocketAddrs) -> io::Result<Self> {
        let mut socket = TcpStream::connect(addr).await?;

        // client connection preface
        socket
            .write_all(b"PRI * HTTP/2.0\r\n\r\nSM\r\n\r\n")
            .await?;

        let frame: Frame = hashmap! {
            SettingsParameter::HeaderTableSize => 0,
            SettingsParameter::EnablePush => 0,
        }
        .into();
        frame.write_into(&mut socket).await?;

        socket.flush().await?;

        Ok(Self {
            socket,
            streams: HashMap::new(),
            their_settings: HashMap::new(),
        })
    }

    pub async fn handle_frame(&mut self) -> anyhow::Result<()> {
        let frame = Frame::read_from(&mut self.socket).await?;
        match frame {
            Frame::Settings { params, .. } => {
                self.their_settings = params;
                Frame::new_settings_ack()
                    .write_into(&mut self.socket)
                    .await?;
            }
            _ => {
                eprintln!("Unhandled frame type: {:#?}", frame);
            }
        }
        Ok(())
    }
}

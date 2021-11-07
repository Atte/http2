use crate::{enums::*, frame::*, stream::Stream};
use async_std::io;
use async_std::{
    io::WriteExt,
    net::{TcpStream, ToSocketAddrs},
};
use maplit::hashmap;
use std::collections::HashMap;

pub struct Connection {
    socket: TcpStream,
    streams: HashMap<u32, Stream>,
    their_settings: HashMap<SettingsParameter, u32>,
}

impl Connection {
    pub async fn connect(addr: impl ToSocketAddrs) -> io::Result<Self> {
        let mut socket = TcpStream::connect(addr).await?;
        // client connection preface
        socket
            .write_all(b"PRI * HTTP/2.0\r\n\r\nSM\r\n\r\n")
            .await?;

        SettingsFrame::from(hashmap! {
            SettingsParameter::HeaderTableSize => 0,
            SettingsParameter::EnablePush => 0,
        })
        .write_into(&mut socket)
        .await?;

        socket.flush().await?;

        Ok(Self {
            socket,
            streams: HashMap::new(),
            their_settings: HashMap::new(),
        })
    }

    pub async fn handle_frame(&mut self) -> io::Result<()> {
        let frame = Frame::try_from_stream(&mut self.socket).await?;
        match frame.typ {
            FrameType::Settings => {
                let frame: SettingsFrame = frame.try_into().expect("invalid settings frame");
                self.their_settings = frame.into();
                SettingsFrame::ack().write_into(&mut self.socket).await?;
            }
            _ => {
                eprintln!("Unhandled frame type: {:#?}", frame.typ);
            }
        }
        Ok(())
    }
}

use anyhow::bail;
use rustls::ClientConnection;
use std::{
    io::{self, Read, Write},
    net::TcpStream,
};

pub struct Socket {
    client: ClientConnection,
    stream: TcpStream,
    read_buffer: Vec<u8>,
    buffer: Vec<u8>,
}

impl Socket {
    pub fn new(client: ClientConnection, stream: TcpStream) -> Self {
        Self {
            client,
            stream,
            read_buffer: Vec::with_capacity(1024),
            buffer: Vec::with_capacity(1024),
        }
    }

    pub fn read_exact_maybe(&mut self, length: usize) -> anyhow::Result<Option<Vec<u8>>> {
        self.process()?;
        if self.buffer.len() >= length {
            let mut remaining = self.buffer.split_off(length);
            std::mem::swap(&mut self.buffer, &mut remaining);
            Ok(Some(remaining))
        } else {
            Ok(None)
        }
    }

    pub fn read_exact_blocking(&mut self, length: usize) -> anyhow::Result<Vec<u8>> {
        loop {
            if let Some(buf) = self.read_exact_maybe(length)? {
                return Ok(buf);
            }
            std::thread::yield_now();
        }
    }

    pub fn write_all(&mut self, buf: &[u8]) -> anyhow::Result<()> {
        self.process()?;
        self.client.writer().write_all(buf)?;
        Ok(())
    }

    fn process(&mut self) -> anyhow::Result<()> {
        if self.client.wants_read() {
            let mut buffer = [0; 1024];
            self.stream.set_nonblocking(true)?;
            if let Err(err) = self.stream.read(&mut buffer) {
                if err.kind() != io::ErrorKind::WouldBlock {
                    bail!(err);
                }
            }
            self.stream.set_nonblocking(false)?;
            self.read_buffer.extend(buffer);

            if !self.read_buffer.is_empty() {
                self.client.read_tls(&mut self.read_buffer.as_slice())?;
                self.client.process_new_packets()?;

                if let Err(err) = self.client.reader().read(&mut buffer) {
                    if err.kind() != io::ErrorKind::WouldBlock {
                        bail!(err);
                    }
                }
            }
        }

        if self.client.wants_write() {
            self.client.write_tls(&mut self.stream)?;
        }

        Ok(())
    }
}

impl Write for Socket {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.client.writer().write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.client.writer().flush()
    }
}

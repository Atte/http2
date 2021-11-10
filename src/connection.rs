use crate::{frame::*, stream_coordinator::*, types::*};
use anyhow::anyhow;
use bytes::{Buf, BufMut, Bytes, BytesMut};
use enum_map::enum_map;
use log::{debug, error, trace, warn};
use std::sync::atomic::{AtomicUsize, Ordering};
use tokio::{
    io::{split, AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
    sync::{
        broadcast,
        mpsc::{channel, Sender},
    },
};
use tokio_rustls::TlsConnector;
use url::Url;

static REQUEST_ID: AtomicUsize = AtomicUsize::new(1);

#[derive(Debug, Clone)]
pub struct Request {
    pub id: usize,
    pub headers: Vec<(String, String)>,
    pub body: Bytes,
}

#[derive(Debug, Clone)]
pub struct Response {
    pub request_id: usize,
    pub headers: Vec<(String, String)>,
    pub body: Bytes,
}

pub struct Connection {
    request_queue: Sender<Request>,
    responses: broadcast::Sender<Response>,
}

impl Request {
    fn into_bytes(
        self,
        header_encoder: &mut hpack::Encoder,
        streams: &mut StreamCoordinator,
        mut buffer: impl BufMut,
    ) {
        let headers: Bytes = header_encoder
            .encode(
                self.headers
                    .iter()
                    .map(|(key, value)| (key.as_bytes(), value.as_bytes())),
            )
            .into();
        streams.with_new_stream(|stream| {
            Frame::Headers {
                stream: stream.id,
                flags: if self.body.is_empty() {
                    HeadersFlags::END_STREAM | HeadersFlags::END_HEADERS
                } else {
                    HeadersFlags::END_HEADERS
                },
                dependency: None,
                exclusive_dependency: None,
                weight: None,
                fragment: headers,
            }
            .into_bytes(&mut buffer);
            if !self.body.is_empty() {
                Frame::Data {
                    stream: stream.id,
                    flags: DataFlags::END_STREAM,
                    flow_control_size: self.body.len() as u32,
                    data: self.body,
                }
                .into_bytes(&mut buffer);
            }
        });
    }
}

impl Connection {
    pub async fn connect(url: Url, connector: &TlsConnector) -> anyhow::Result<Self> {
        let (mut reader, mut writer) = split(
            connector
                .connect(
                    url.host_str()
                        .ok_or_else(|| anyhow!("connect host name"))?
                        .try_into()
                        .expect("connect host name into server name"),
                    TcpStream::connect(url.socket_addrs(|| None)?[0]).await?,
                )
                .await?,
        );

        // client connection preface
        writer
            .write_all(b"PRI * HTTP/2.0\r\n\r\nSM\r\n\r\n")
            .await?;

        let (request_tx, mut request_rx) = channel::<Request>(100);
        let (response_tx, _response_rx) = broadcast::channel::<Response>(100);
        let responses = response_tx.clone();

        tokio::spawn(async move {
            let mut their_settings = enum_map! {
                SettingsParameter::HeaderTableSize => 4096,
                SettingsParameter::EnablePush => 1,
                SettingsParameter::MaxConcurrentStreams => u32::MAX,
                SettingsParameter::InitialWindowSize => 65_535,
                SettingsParameter::MaxFrameSize => 16_384,
                SettingsParameter::MaxHeaderListSize => u32::MAX,
            };
            let mut streams = StreamCoordinator::default();
            let mut window_remaining: usize = 65_535;

            let mut header_encoder = hpack::Encoder::new();
            let mut header_decoder = hpack::Decoder::new();
            let mut read_buf = BytesMut::with_capacity(16_384 * 2);
            let mut write_buf = BytesMut::with_capacity(16_384 * 2);
            let mut header: Bytes = Bytes::new();
            let mut ready = false;

            loop {
                let was_ready = ready;
                let mut handle_frame = |frame: Frame,
                                        write_buf: &mut BytesMut|
                 -> anyhow::Result<Option<Response>> {
                    Ok(match frame {
                        Frame::Data { stream, .. } => streams.with_stream(stream, |stream| {
                            stream.on_frame(frame, write_buf, &mut header_decoder)
                        })?,
                        Frame::Headers { stream, .. } => streams.with_stream(stream, |stream| {
                            stream.on_frame(frame, write_buf, &mut header_decoder)
                        })?,
                        Frame::Priority { stream, .. } => streams
                            .with_stream(stream, |stream| {
                                stream.on_frame(frame, write_buf, &mut header_decoder)
                            })?,
                        Frame::ResetStream { stream, .. } => streams
                            .with_stream(stream, |stream| {
                                stream.on_frame(frame, write_buf, &mut header_decoder)
                            })?,
                        Frame::Settings { params, .. } => {
                            for (key, value) in params {
                                their_settings[key] = value;
                            }

                            if !ready {
                                let settings_frame: Frame = vec![
                                    (SettingsParameter::EnablePush, 0),
                                    (SettingsParameter::InitialWindowSize, U31_MAX.get()),
                                ]
                                .into();
                                settings_frame.into_bytes(write_buf);
                                ready = true;
                            }

                            Frame::Settings {
                                flags: SettingsFlags::ACK,
                                params: Vec::new(),
                            }
                            .into_bytes(write_buf);

                            None
                        }
                        Frame::PushPromise { stream, .. } => streams
                            .with_stream(stream, |stream| {
                                stream.on_frame(frame, write_buf, &mut header_decoder)
                            })?,
                        Frame::Ping { flags, data, .. } => {
                            if !flags.contains(PingFlags::ACK) {
                                if data.len() == 8 {
                                    Frame::Ping {
                                        flags: PingFlags::ACK,
                                        data,
                                    }
                                    .into_bytes(write_buf);
                                } else {
                                    Frame::GoAway {
                                        last_stream: 0,
                                        error: ErrorType::ProtocolError,
                                        debug: Bytes::from_static(b"invalid ping payload length"),
                                    }
                                    .into_bytes(write_buf);
                                }
                            }
                            None
                        }
                        Frame::GoAway { error, debug, .. } => {
                            error!("Go away: {:?}", error);
                            if !debug.is_empty() {
                                if let Ok(debug) = std::str::from_utf8(&debug) {
                                    debug!("Go away debug: {}", debug);
                                }
                            }
                            None
                        }
                        Frame::WindowUpdate {
                            stream, increment, ..
                        } => {
                            if let Some(stream) = NonZeroStreamId::new(stream) {
                                streams.with_stream(stream, |stream| {
                                    stream.on_frame(frame, write_buf, &mut header_decoder)
                                })?
                            } else {
                                window_remaining =
                                    window_remaining.saturating_add(increment.get() as usize);
                                None
                            }
                        }
                        Frame::Continuation { stream, .. } => streams
                            .with_stream(stream, |stream| {
                                stream.on_frame(frame, write_buf, &mut header_decoder)
                            })?,
                    })
                };

                /*
                if write_buf.has_remaining() {
                    trace!("write {:?}", write_buf);
                }
                */

                tokio::select! {
                    res = reader.read_buf(&mut read_buf) => {
                        res.expect("read_buf");
                        /*
                        if read_buf.has_remaining() {
                            trace!("read {:?}", read_buf);
                        }
                        */
                        loop {
                            if header.is_empty() {
                                if let Some(new_header) = Frame::try_header_from_bytes(&mut read_buf) {
                                    header = new_header;
                                } else {
                                    break;
                                }
                            } else if let Some(frame) = Frame::try_read_with_header(&header, &mut read_buf).expect("read_with_header") {
                                header = Bytes::new();
                                match handle_frame(frame, &mut write_buf) {
                                    err @ Err(_) => {
                                        err.expect("handle_frame");
                                    },
                                    Ok(Some(response)) => {
                                        trace!("{:#?}", response);
                                        if let Err(err) = response_tx.send(response) {
                                            warn!("Error broadcasting response: {:#?}", err);
                                        }
                                    },
                                    Ok(None) => {}
                                }
                            } else {
                                break;
                            }
                        }
                    }
                    res = writer.write_buf(&mut write_buf), if write_buf.has_remaining() => {
                        res.expect("write_buf");
                    }
                    request = request_rx.recv(), if was_ready => {
                        if let Some(request) = request {
                            trace!("{:#?}", request);
                            request.into_bytes(&mut header_encoder, &mut streams, &mut write_buf);
                        } else {
                            trace!("Closing connection...");
                            return;
                        }
                    }
                }
            }
        });

        Ok(Self {
            request_queue: request_tx,
            responses,
        })
    }

    pub async fn request(
        &self,
        headers: Vec<(String, String)>,
        body: impl Into<Bytes>,
    ) -> anyhow::Result<Response> {
        let id = REQUEST_ID.fetch_add(1, Ordering::SeqCst);

        let mut receiver = self.responses.subscribe();

        self.request_queue
            .send(Request {
                id,
                headers: headers.clone(),
                body: body.into(),
            })
            .await?;

        loop {
            let response = receiver.recv().await?;
            if response.request_id == id {
                return Ok(response);
            }
        }
    }
}

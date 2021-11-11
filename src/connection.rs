use crate::{flags::*, frame::*, stream_coordinator::*, types::*};
use anyhow::anyhow;
use bytes::{Buf, Bytes, BytesMut};
use derivative::Derivative;
use enum_map::{enum_map, EnumMap};
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

#[derive(Derivative)]
#[derivative(Debug)]
pub struct ConnectionState {
    pub their_settings: EnumMap<SettingsParameter, u32>,
    pub window_remaining: usize,
    #[derivative(Debug = "ignore")]
    pub header_encoder: hpack::Encoder<'static>,
    #[derivative(Debug = "ignore")]
    pub header_decoder: hpack::Decoder<'static>,
    pub read_buf: BytesMut,
    pub write_buf: BytesMut,
    pub header: Option<FrameHeader>,
    pub ready: bool,
}

impl Default for ConnectionState {
    fn default() -> Self {
        Self {
            their_settings: enum_map! {
                SettingsParameter::HeaderTableSize => 4096,
                SettingsParameter::EnablePush => 1,
                SettingsParameter::MaxConcurrentStreams => u32::MAX,
                SettingsParameter::InitialWindowSize => 65_535,
                SettingsParameter::MaxFrameSize => 16_384,
                SettingsParameter::MaxHeaderListSize => u32::MAX,
            },
            window_remaining: 65_535,
            header_encoder: hpack::Encoder::new(),
            header_decoder: hpack::Decoder::new(),
            read_buf: BytesMut::with_capacity(16_384 + FrameHeader::BYTES),
            write_buf: BytesMut::with_capacity(16_384 + FrameHeader::BYTES),
            header: None,
            ready: false,
        }
    }
}

pub struct Connection {
    requests: Sender<Request>,
    responses: broadcast::Sender<Response>,
}

impl Request {
    fn write_into(self, state: &mut ConnectionState, streams: &mut StreamCoordinator) {
        let stream = streams.create_mut();
        stream.request_id = self.id;
        FramePayload::Headers {
            dependency: None,
            exclusive_dependency: None,
            weight: None,
            fragment: state
                .header_encoder
                .encode(
                    self.headers
                        .iter()
                        .map(|(key, value)| (key.as_bytes(), value.as_bytes())),
                )
                .into(),
        }
        .write_into(
            &mut state.write_buf,
            Some(stream),
            if self.body.is_empty() {
                HeadersFlags::END_STREAM | HeadersFlags::END_HEADERS
            } else {
                HeadersFlags::END_HEADERS
            },
        );
        if !self.body.is_empty() {
            FramePayload::Data { data: self.body }.write_into(
                &mut state.write_buf,
                Some(stream),
                DataFlags::END_STREAM,
            );
        }
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
            let mut state = ConnectionState::default();
            let mut streams = StreamCoordinator::default();

            loop {
                let was_ready = state.ready;
                tokio::select! {
                    res = reader.read_buf(&mut state.read_buf) => {
                        res.expect("read_buf");
                        loop {
                            if let Some(ref header) = state.header {
                                match FramePayload::try_from(&mut state.read_buf, header) {
                                    Ok(payload) => {
                                        match Self::handle_frame(&mut state, &mut streams, payload) {
                                            Ok(Some(response)) => {
                                                trace!("{:#?}", response);
                                                if let Err(err) = response_tx.send(response) {
                                                    warn!("Error broadcasting response: {:#?}", err);
                                                }
                                            },
                                            Ok(None) => {}
                                            err @ Err(_) => {
                                                err.expect("handle_frame");
                                            },
                                        }
                                        state.header = None;
                                    },
                                    Err(FrameDecodeError::TooShort) => {}
                                    err @ Err(_) => {
                                        err.expect("FramePayload::try_from");
                                    },
                                }
                            } else {
                                match FrameHeader::try_from(&mut state.read_buf) {
                                  Ok(header) => { state.header = Some(header); }
                                  Err(FrameDecodeError::TooShort) => { break; }
                                  err @ Err(_) => {
                                    err.expect("FrameHeader::try_from");
                                  }
                                }
                            }
                        }
                    }
                    res = writer.write_buf(&mut state.write_buf), if state.write_buf.has_remaining() => {
                        res.expect("write_buf");
                    }
                    request = request_rx.recv(), if was_ready => {
                        if let Some(request) = request {
                            trace!("{:#?}", request);
                            request.write_into(&mut state, &mut streams);
                        } else {
                            return;
                        }
                    }
                }
            }
        });

        Ok(Self {
            requests: request_tx,
            responses,
        })
    }

    fn handle_frame(
        state: &mut ConnectionState,
        streams: &mut StreamCoordinator,
        payload: FramePayload,
    ) -> anyhow::Result<Option<Response>> {
        let header = state.header.as_ref().expect("no header for payload");
        Ok(match (header.flags, payload) {
            (Flags::Settings(flags), FramePayload::Settings { params, .. }) => {
                if !flags.contains(SettingsFlags::ACK) {
                    for (key, value) in params {
                        state.their_settings[key] = value;
                    }
                    if !state.ready {
                        FramePayload::Settings {
                            params: vec![(SettingsParameter::InitialWindowSize, U31_MAX.get())],
                        }
                        .write_into(
                            &mut state.write_buf,
                            None,
                            Flags::None,
                        );
                        state.ready = true;
                    }
                    FramePayload::Settings { params: Vec::new() }.write_into(
                        &mut state.write_buf,
                        None,
                        SettingsFlags::ACK,
                    );
                }
                None
            }
            (Flags::Ping(flags), FramePayload::Ping { data, .. }) => {
                if !flags.contains(PingFlags::ACK) {
                    if data.len() == 8 {
                        FramePayload::Ping { data }.write_into(
                            &mut state.write_buf,
                            None,
                            PingFlags::ACK,
                        );
                    } else {
                        FramePayload::GoAway {
                            last_stream: 0,
                            error: ErrorType::ProtocolError,
                            debug: Bytes::from_static(b"invalid ping payload length"),
                        }
                        .write_into(
                            &mut state.write_buf,
                            None,
                            Flags::None,
                        );
                    }
                }
                None
            }
            (_, FramePayload::GoAway { error, debug, .. }) => {
                error!("Go away: {:?}", error);
                if !debug.is_empty() {
                    if let Ok(debug) = std::str::from_utf8(&debug) {
                        debug!("Go away debug: {}", debug);
                    }
                }
                None
            }
            (_, FramePayload::WindowUpdate { increment, .. }) => {
                if let Some(stream_id) = NonZeroStreamId::new(header.stream_id) {
                    streams
                        .get_mut(stream_id)
                        .handle_frame(state, FramePayload::WindowUpdate { increment })?
                } else {
                    state.window_remaining = state
                        .window_remaining
                        .saturating_add(increment.get() as usize);
                    None
                }
            }
            (
                _,
                FramePayload::PushPromise {
                    promised_stream,
                    fragment,
                },
            ) => {
                let stream = streams.get_mut(promised_stream);
                stream.handle_frame(
                    state,
                    FramePayload::PushPromise {
                        promised_stream,
                        fragment,
                    },
                )?;
                None
            }
            (_, payload) => streams
                .get_mut(
                    NonZeroStreamId::new(header.stream_id).ok_or(FrameDecodeError::ZeroStreamId)?,
                )
                .handle_frame(state, payload)?,
        })
    }

    pub async fn request(
        &self,
        headers: Vec<(String, String)>,
        body: impl Into<Bytes>,
    ) -> anyhow::Result<Response> {
        let id = REQUEST_ID.fetch_add(1, Ordering::SeqCst);

        let mut receiver = self.responses.subscribe();

        self.requests
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

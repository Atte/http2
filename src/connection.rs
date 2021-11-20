use crate::{
    flags::*, frame::*, request::Request, response::Response, stream_coordinator::*, types::*,
};
use anyhow::anyhow;
use bytes::{Buf, Bytes, BytesMut};
use derivative::Derivative;
use enum_map::{enum_map, EnumMap};
use log::{debug, error, trace};
use tokio::{
    io::{split, AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
    sync::{mpsc, oneshot},
};
use tokio_rustls::TlsConnector;
use url::Url;

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
    #[must_use]
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
            read_buf: BytesMut::with_capacity(16_384 + FrameHeader::SIZE),
            write_buf: BytesMut::with_capacity(16_384 + FrameHeader::SIZE),
            header: None,
            ready: false,
        }
    }
}

static CLIENT_CONNECTION_PREFACE: &[u8] = b"PRI * HTTP/2.0\r\n\r\nSM\r\n\r\n";

pub struct Connection {
    requests: mpsc::Sender<(Request, oneshot::Sender<Response>)>,
}

impl Connection {
    pub async fn connect(url: &Url, connector: &TlsConnector) -> anyhow::Result<Self> {
        let mut early_data_sent = false;
        let mut stream = connector
            .connect_with(
                url.host_str()
                    .ok_or_else(|| anyhow!("connect host name"))?
                    .try_into()
                    .map_err(|err| anyhow!("connect host name into server name: {:?}", err))?,
                TcpStream::connect(url.socket_addrs(|| None)?[0]).await?,
                |connection| {
                    use std::io::Write;
                    if let Some(mut early) = connection.early_data() {
                        if early.bytes_left() >= CLIENT_CONNECTION_PREFACE.len() {
                            if let Err(err) = early.write_all(CLIENT_CONNECTION_PREFACE) {
                                error!("Failed to write early data: {:?}", err);
                            } else {
                                early_data_sent = true;
                            }
                        }
                    }
                },
            )
            .await?;

        if !early_data_sent || !stream.get_ref().1.is_early_data_accepted() {
            stream.write_all(CLIENT_CONNECTION_PREFACE).await?;
        }

        let (mut reader, mut writer) = split(stream);
        let (requests_tx, mut requests_rx) =
            mpsc::channel::<(Request, oneshot::Sender<Response>)>(16);

        tokio::spawn(async move {
            let mut state = ConnectionState::default();
            let mut streams = StreamCoordinator::default();

            loop {
                tokio::select! {
                    res = reader.read_buf(&mut state.read_buf) => {
                        res.expect("read_buf");
                        loop {
                            if let Some(ref header) = state.header {
                                match FramePayload::try_from(&mut state.read_buf, header) {
                                    Ok(payload) => {
                                        Self::handle_frame(&mut state, &mut streams, payload).expect("handle_frame");
                                        state.header = None;
                                    },
                                    Err(FrameDecodeError::TooShort) => {
                                        break;
                                    }
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
                    entry = requests_rx.recv(), if state.ready => {
                        if let Some((request, response_tx)) = entry {
                            trace!("{:#?}", request);
                            request.write_into(&mut state, &mut streams, response_tx);
                        } else {
                            return;
                        }
                    }
                }
            }
        });

        Ok(Self {
            requests: requests_tx,
        })
    }

    fn handle_frame(
        state: &mut ConnectionState,
        streams: &mut StreamCoordinator,
        payload: FramePayload,
    ) -> anyhow::Result<()> {
        let header = state.header.as_ref().expect("no header for payload");
        match (header.flags, payload) {
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
            }
            (_, FramePayload::GoAway { error, debug, .. }) => {
                error!("Go away: {:?}", error);
                if !debug.is_empty() {
                    if let Ok(debug) = std::str::from_utf8(&debug) {
                        debug!("Go away debug: {}", debug);
                    }
                }
            }
            (_, FramePayload::WindowUpdate { increment, .. }) => {
                if let Some(stream_id) = NonZeroStreamId::new(header.stream_id) {
                    streams
                        .get_mut(stream_id)
                        .handle_frame(state, FramePayload::WindowUpdate { increment })?;
                } else {
                    state.window_remaining = state
                        .window_remaining
                        .saturating_add(increment.get() as usize);
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
            }
            (_, payload) => {
                streams
                    .get_mut(
                        NonZeroStreamId::new(header.stream_id)
                            .ok_or(FrameDecodeError::ZeroStreamId)?,
                    )
                    .handle_frame(state, payload)?;
            }
        }
        Ok(())
    }

    pub async fn request(&self, request: Request) -> anyhow::Result<Response> {
        let (tx, rx) = oneshot::channel();
        self.requests.send((request, tx)).await?;
        Ok(rx.await?)
    }
}

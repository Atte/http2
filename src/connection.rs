#![allow(clippy::mutex_atomic)] // needed for Condvar

use crate::{frame::*, socket::Socket, stream_coordinator::*, types::*};
use anyhow::anyhow;
use enum_map::{enum_map, EnumMap};
use log::{debug, error, trace};
use std::{
    collections::HashMap,
    io::{Read, Write},
    net::TcpStream,
    sync::{
        mpsc::{channel, Receiver, Sender},
        Arc, Condvar, Mutex, RwLock,
    },
    thread,
};
use url::Url;

#[derive(Debug, Clone)]
pub struct Request {
    pub headers: HashMap<String, String>,
    pub body: Vec<u8>,
}

#[derive(Debug, Clone)]
pub struct Response {
    pub request_headers: HashMap<String, String>,
    pub headers: HashMap<String, String>,
    pub body: Vec<u8>,
}

pub struct Connection {
    request_queue: Sender<Request>,
    responses: Arc<(Mutex<Vec<Response>>, Condvar)>,
}

fn spawn_request_sender(
    ready: Arc<(Mutex<bool>, Condvar)>,
    request_rx: Receiver<Request>,
    socket: Arc<Mutex<Socket>>,
    streams: Arc<StreamCoordinator>,
    _their_settings: Arc<RwLock<EnumMap<SettingsParameter, u32>>>,
) {
    thread::spawn(move || {
        let _guard = ready
            .1
            .wait_while(ready.0.lock().expect("ready lock"), |ready| !*ready)
            .expect("wait for ready");

        let settings_frame: Frame = vec![(SettingsParameter::EnablePush, 0)].into();
        settings_frame.write_into(&socket).expect("settings frame");

        let mut header_encoder = hpack::Encoder::new();

        let mut handle_request = |request: Request| {
            let headers = header_encoder.encode(
                request
                    .headers
                    .iter()
                    .map(|(key, value)| (key.as_bytes(), value.as_bytes())),
            );
            streams.with_new_stream(|mut stream| -> anyhow::Result<()> {
                stream.request_headers = request.headers;
                Frame::Headers {
                    stream: stream.id,
                    flags: if request.body.is_empty() {
                        HeadersFlags::END_STREAM | HeadersFlags::END_HEADERS
                    } else {
                        HeadersFlags::END_HEADERS
                    },
                    dependency: 0,
                    exclusive_dependency: false,
                    weight: 0,
                    fragment: headers,
                }
                .write_into(&socket)?;
                if !request.body.is_empty() {
                    Frame::Data {
                        stream: stream.id,
                        flags: DataFlags::END_STREAM,
                        data: request.body,
                    }
                    .write_into(&socket)?;
                }
                Ok(())
            })
        };

        loop {
            match request_rx.recv() {
                Ok(request) => {
                    handle_request(request).expect("handle_request");
                }
                Err(err) => {
                    error!("request_rx.recv(): {:#?}", err);
                    // TODO: handle closing
                    return;
                }
            }
        }
    });
}

fn spawn_response_receiver(
    ready: Arc<(Mutex<bool>, Condvar)>,
    responses: Arc<(Mutex<Vec<Response>>, Condvar)>,
    socket: Arc<Mutex<Socket>>,
    streams: Arc<StreamCoordinator>,
    their_settings: Arc<RwLock<EnumMap<SettingsParameter, u32>>>,
) {
    thread::spawn(move || {
        let mut window_remaining: u64 = 65_535;
        let mut header_decoder = hpack::Decoder::new();

        let mut handle_frame = |frame: Frame| -> anyhow::Result<Option<Response>> {
            Ok(match frame {
                Frame::Data { stream, .. } => streams.with_stream(stream, |stream| {
                    stream.on_frame(frame, &socket, &mut header_decoder)
                })?,
                Frame::Headers { stream, .. } => streams.with_stream(stream, |stream| {
                    stream.on_frame(frame, &socket, &mut header_decoder)
                })?,
                Frame::Priority { stream, .. } => streams.with_stream(stream, |stream| {
                    stream.on_frame(frame, &socket, &mut header_decoder)
                })?,
                Frame::ResetStream { stream, .. } => streams.with_stream(stream, |stream| {
                    stream.on_frame(frame, &socket, &mut header_decoder)
                })?,
                Frame::Settings { params, .. } => {
                    let mut their_settings = their_settings
                        .write()
                        .expect("response_their_settings write");
                    for (key, value) in params {
                        their_settings[key] = value;
                    }
                    Frame::Settings {
                        flags: SettingsFlags::ACK,
                        params: Vec::new(),
                    }
                    .write_into(&socket)?;

                    let mut is_ready = ready.0.lock().expect("ready lock");
                    if !*is_ready {
                        trace!("ready!");
                        *is_ready = true;
                        ready.1.notify_all();
                    }

                    None
                }
                Frame::PushPromise { stream, .. } => streams.with_stream(stream, |stream| {
                    stream.on_frame(frame, &socket, &mut header_decoder)
                })?,
                Frame::Ping { flags, data, .. } => {
                    if !flags.contains(PingFlags::ACK) {
                        if data.len() == 8 {
                            Frame::Ping {
                                flags: PingFlags::ACK,
                                data,
                            }
                            .write_into(&socket)?;
                        } else {
                            Frame::GoAway {
                                last_stream: 0,
                                error: ErrorType::ProtocolError,
                                debug: "invalid ping payload length".as_bytes().to_vec(),
                            }
                            .write_into(&socket)?;
                        }
                    }
                    None
                }
                Frame::GoAway { error, debug, .. } => {
                    error!("Go away: {:?}", error);
                    if !debug.is_empty() {
                        if let Ok(debug) = String::from_utf8(debug) {
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
                            stream.on_frame(frame, &socket, &mut header_decoder)
                        })?
                    } else {
                        window_remaining = window_remaining.saturating_add(increment.get() as u64);
                        None
                    }
                }
                Frame::Continuation { stream, .. } => streams.with_stream(stream, |stream| {
                    stream.on_frame(frame, &socket, &mut header_decoder)
                })?,
            })
        };

        loop {
            // TODO: handle connection loss
            if let Some(frame) = Frame::read_from(&socket).expect("frame read_from") {
                if let Some(response) = handle_frame(frame).expect("handle_frame") {
                    responses.0.lock().expect("responses lock").push(response);
                    responses.1.notify_all();
                }
            }
        }
    });
}

impl Connection {
    pub fn connect(url: Url, rustls_config: Arc<rustls::ClientConfig>) -> anyhow::Result<Self> {
        let mut socket = Socket::new(
            rustls::ClientConnection::new(
                rustls_config,
                url.host_str()
                    .ok_or_else(|| anyhow!("connect host name"))?
                    .try_into()
                    .expect("connect host name into server name"),
            )?,
            TcpStream::connect(url.socket_addrs(|| None)?[0])?,
        );

        // client connection preface
        socket.write_all(b"PRI * HTTP/2.0\r\n\r\nSM\r\n\r\n")?;
        let socket = Arc::new(Mutex::new(socket));

        let (request_tx, request_rx) = channel::<Request>();
        let responses = Arc::new((Mutex::new(Vec::new()), Condvar::new()));

        let ready = Arc::new((Mutex::new(false), Condvar::new()));
        let streams = Arc::new(StreamCoordinator::default());
        let their_settings = Arc::new(RwLock::new(enum_map! {
            SettingsParameter::HeaderTableSize => 4096,
            SettingsParameter::EnablePush => 1,
            SettingsParameter::MaxConcurrentStreams => u32::MAX,
            SettingsParameter::InitialWindowSize => 65_535,
            SettingsParameter::MaxFrameSize => 16_384,
            SettingsParameter::MaxHeaderListSize => u32::MAX,
        }));

        spawn_request_sender(
            ready.clone(),
            request_rx,
            socket.clone(),
            streams.clone(),
            their_settings.clone(),
        );

        spawn_response_receiver(ready, responses.clone(), socket, streams, their_settings);

        Ok(Self {
            request_queue: request_tx,
            responses,
        })
    }

    pub fn request(&self, headers: HashMap<String, String>, body: Vec<u8>) -> Response {
        self.request_queue
            .send(Request {
                headers: headers.clone(),
                body,
            })
            .expect("request_queue send");

        loop {
            let mut responses = self
                .responses
                .1
                .wait(self.responses.0.lock().expect("responses write"))
                .expect("responses wait");
            if let Some(index) = responses
                .iter()
                .position(|response| response.request_headers == headers)
            {
                return responses.remove(index);
            }
        }
    }
}

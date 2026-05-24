use std::ffi::OsString;
use std::io::{BufReader, Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;

use kameo::actor::{Actor, ActorRef, Spawn};
use kameo::error::Infallible;
use kameo::message::{Context, Message};
use signal_core::{ExchangeIdentifier, NonEmpty, Reply, SignalVerb, SubReply};
use signal_system::{
    SystemBackend, SystemDaemonConfiguration, SystemFrame, SystemFrameBody as FrameBody,
    SystemHealth, SystemOperationKind, SystemReadiness, SystemReply, SystemRequest,
    SystemRequestUnimplemented, SystemStatus, SystemStatusQuery, SystemUnimplementedReason,
};

use crate::error::{Error, Result};
use crate::supervision::{SupervisionListener, SupervisionProfile, SupervisionSocketMode};

#[derive(Debug)]
pub struct SystemDaemon {
    socket: PathBuf,
    backend: SystemBackend,
    socket_mode: Option<SocketMode>,
    supervision: Option<SupervisionListener>,
}

impl SystemDaemon {
    /// Canonical constructor — every production launch reads typed
    /// `SystemDaemonConfiguration` from argv via `nota-config` and
    /// hands the record here.
    pub fn from_configuration(configuration: SystemDaemonConfiguration) -> Self {
        let supervision = SupervisionListener::new(
            SupervisionProfile::system(),
            PathBuf::from(configuration.supervision_socket_path.as_str()),
            SupervisionSocketMode::from_octal(configuration.supervision_socket_mode.into_u32()),
        );
        Self {
            socket: PathBuf::from(configuration.system_socket_path.as_str()),
            backend: configuration.backend,
            socket_mode: Some(SocketMode::from_octal(
                configuration.system_socket_mode.into_u32(),
            )),
            supervision: Some(supervision),
        }
    }

    pub fn from_socket(socket: impl Into<PathBuf>) -> Self {
        Self {
            socket: socket.into(),
            backend: SystemBackend::Niri,
            socket_mode: SocketMode::from_environment(),
            supervision: None,
        }
    }

    pub fn with_backend(mut self, backend: SystemBackend) -> Self {
        self.backend = backend;
        self
    }

    pub fn with_socket_mode(mut self, socket_mode: SocketMode) -> Self {
        self.socket_mode = Some(socket_mode);
        self
    }

    pub fn socket(&self) -> &PathBuf {
        &self.socket
    }

    pub fn backend(&self) -> SystemBackend {
        self.backend
    }

    pub fn run(self) -> Result<()> {
        let supervision = self.supervision.clone();
        let bound = self.bind()?;
        let _supervision = supervision.map(SupervisionListener::spawn).transpose()?;
        eprintln!("system-daemon socket={}", bound.socket.display());
        bound.serve_forever()
    }

    pub fn bind(self) -> Result<BoundSystemDaemon> {
        if let Some(parent) = self.socket.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let _ = std::fs::remove_file(&self.socket);
        let listener = UnixListener::bind(&self.socket)?;
        if let Some(socket_mode) = self.socket_mode {
            std::fs::set_permissions(
                &self.socket,
                std::fs::Permissions::from_mode(socket_mode.as_octal()),
            )?;
        }
        let runtime = tokio::runtime::Runtime::new()?;
        let system = runtime.block_on(SystemSupervisor::start(self.backend));
        Ok(BoundSystemDaemon {
            socket: self.socket,
            runtime,
            listener,
            system,
        })
    }

    pub fn serve_one(self) -> Result<SystemReply> {
        self.bind()?.serve_one()
    }

    fn handle_connection(
        runtime: &tokio::runtime::Runtime,
        system: &ActorRef<SystemSupervisor>,
        stream: UnixStream,
    ) -> Result<SystemReply> {
        let mut connection = SystemConnection::from_stream(stream);
        let request = connection.read_signal_request()?;
        let reply = runtime.block_on(async {
            SystemRequestHandler::new(system.clone())
                .reply_for_request(request.request)
                .await
        })?;
        connection.write_signal_reply(request.exchange, request.verb, reply.clone())?;
        Ok(reply)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SocketMode(u32);

impl SocketMode {
    pub const fn from_octal(value: u32) -> Self {
        Self(value)
    }

    pub fn from_environment() -> Option<Self> {
        std::env::var("PERSONA_SOCKET_MODE")
            .ok()
            .and_then(|value| u32::from_str_radix(value.as_str(), 8).ok())
            .map(Self::from_octal)
    }

    pub const fn as_octal(self) -> u32 {
        self.0
    }
}

pub struct BoundSystemDaemon {
    socket: PathBuf,
    runtime: tokio::runtime::Runtime,
    listener: UnixListener,
    system: ActorRef<SystemSupervisor>,
}

impl BoundSystemDaemon {
    pub fn socket(&self) -> &PathBuf {
        &self.socket
    }

    pub fn serve_one(self) -> Result<SystemReply> {
        let (stream, _address) = self.listener.accept()?;
        let reply = SystemDaemon::handle_connection(&self.runtime, &self.system, stream)?;
        self.runtime.block_on(SystemSupervisor::stop(self.system))?;
        let _ = std::fs::remove_file(&self.socket);
        Ok(reply)
    }

    pub fn serve_forever(self) -> Result<()> {
        for stream in self.listener.incoming() {
            let stream = stream?;
            let _ = SystemDaemon::handle_connection(&self.runtime, &self.system, stream)?;
        }
        Ok(())
    }
}

pub struct SystemConnection {
    stream: BufReader<UnixStream>,
    signal: SystemFrameCodec,
}

impl SystemConnection {
    pub fn from_stream(stream: UnixStream) -> Self {
        Self {
            stream: BufReader::new(stream),
            signal: SystemFrameCodec::default(),
        }
    }

    pub fn read_signal_request(&mut self) -> Result<ReceivedSystemRequest> {
        self.signal.read_request(&mut self.stream)
    }

    pub fn write_signal_reply(
        &mut self,
        exchange: ExchangeIdentifier,
        verb: SignalVerb,
        reply: SystemReply,
    ) -> Result<()> {
        let stream = self.stream.get_mut();
        self.signal.write_reply(stream, exchange, verb, reply)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SystemFrameCodec {
    maximum_frame_bytes: usize,
}

impl SystemFrameCodec {
    pub const fn new(maximum_frame_bytes: usize) -> Self {
        Self {
            maximum_frame_bytes,
        }
    }

    pub fn read_frame(&self, reader: &mut impl Read) -> Result<SystemFrame> {
        let mut prefix = [0_u8; 4];
        reader.read_exact(&mut prefix)?;
        let length = u32::from_be_bytes(prefix) as usize;
        if length > self.maximum_frame_bytes {
            return Err(Error::UnexpectedSignalFrame {
                got: format!("frame length {length} exceeds {}", self.maximum_frame_bytes),
            });
        }
        let mut bytes = Vec::with_capacity(4 + length);
        bytes.extend_from_slice(&prefix);
        bytes.resize(4 + length, 0);
        reader.read_exact(&mut bytes[4..])?;
        Ok(SystemFrame::decode_length_prefixed(&bytes)?)
    }

    pub fn read_request(&self, reader: &mut impl Read) -> Result<ReceivedSystemRequest> {
        match self.read_frame(reader)?.into_body() {
            FrameBody::Request { exchange, request } => {
                let checked = request.into_checked().map_err(|(error, _request)| {
                    Error::UnexpectedSignalFrame {
                        got: error.to_string(),
                    }
                })?;
                let (operation, tail) = checked.operations.into_head_and_tail();
                if !tail.is_empty() {
                    return Err(Error::UnexpectedSignalFrame {
                        got: format!("expected one system operation, got {}", tail.len() + 1),
                    });
                }
                Ok(ReceivedSystemRequest {
                    exchange,
                    verb: operation.verb,
                    request: operation.payload,
                })
            }
            other => Err(Error::UnexpectedSignalFrame {
                got: format!("{other:?}"),
            }),
        }
    }

    pub fn write_reply(
        &self,
        writer: &mut impl Write,
        exchange: ExchangeIdentifier,
        verb: SignalVerb,
        system_reply: SystemReply,
    ) -> Result<()> {
        let frame = SystemFrame::new(FrameBody::Reply {
            exchange,
            reply: Reply::completed(NonEmpty::single(SubReply::Ok {
                verb,
                payload: system_reply,
            })),
        });
        let bytes = frame.encode_length_prefixed()?;
        writer.write_all(&bytes)?;
        writer.flush()?;
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReceivedSystemRequest {
    exchange: ExchangeIdentifier,
    verb: SignalVerb,
    request: SystemRequest,
}

impl Default for SystemFrameCodec {
    fn default() -> Self {
        Self::new(1024 * 1024)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, kameo::Reply)]
pub struct SystemState {
    pub backend: SystemBackend,
    pub served_request_count: u64,
    pub last_operation: Option<SystemOperationKind>,
}

#[derive(Debug)]
pub struct SystemSupervisor {
    backend: SystemBackend,
    served_request_count: u64,
    last_operation: Option<SystemOperationKind>,
}

impl SystemSupervisor {
    pub fn new(backend: SystemBackend) -> Self {
        Self {
            backend,
            served_request_count: 0,
            last_operation: None,
        }
    }

    pub async fn start(backend: SystemBackend) -> ActorRef<Self> {
        let reference = Self::spawn(backend);
        reference.wait_for_startup().await;
        reference
    }

    pub async fn stop(reference: ActorRef<Self>) -> Result<()> {
        reference
            .stop_gracefully()
            .await
            .map_err(|error| Error::ActorCall {
                detail: error.to_string(),
            })?;
        reference.wait_for_shutdown().await;
        Ok(())
    }

    fn state(&self) -> SystemState {
        SystemState {
            backend: self.backend,
            served_request_count: self.served_request_count,
            last_operation: self.last_operation,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReadSystemState {
    pub minimum_served_request_count: u64,
}

impl ReadSystemState {
    pub fn expecting_at_least(minimum_served_request_count: u64) -> Self {
        Self {
            minimum_served_request_count,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RecordServedSystemRequest {
    pub operation: SystemOperationKind,
}

impl Actor for SystemSupervisor {
    type Args = SystemBackend;
    type Error = Infallible;

    async fn on_start(
        backend: Self::Args,
        _actor_reference: ActorRef<Self>,
    ) -> std::result::Result<Self, Self::Error> {
        Ok(Self::new(backend))
    }
}

impl Message<ReadSystemState> for SystemSupervisor {
    type Reply = SystemState;

    async fn handle(
        &mut self,
        message: ReadSystemState,
        _context: &mut Context<Self, Self::Reply>,
    ) -> Self::Reply {
        let _satisfied = self.served_request_count >= message.minimum_served_request_count;
        self.state()
    }
}

impl Message<RecordServedSystemRequest> for SystemSupervisor {
    type Reply = SystemState;

    async fn handle(
        &mut self,
        message: RecordServedSystemRequest,
        _context: &mut Context<Self, Self::Reply>,
    ) -> Self::Reply {
        self.last_operation = Some(message.operation);
        self.served_request_count = self.served_request_count.saturating_add(1);
        self.state()
    }
}

#[derive(Debug, Clone)]
pub struct SystemRequestHandler {
    system: ActorRef<SystemSupervisor>,
}

impl SystemRequestHandler {
    pub fn new(system: ActorRef<SystemSupervisor>) -> Self {
        Self { system }
    }

    pub async fn reply_for_request(&self, request: SystemRequest) -> Result<SystemReply> {
        let operation = request.operation_kind();
        let _state = self
            .system
            .ask(RecordServedSystemRequest { operation })
            .await
            .map_err(|error| Error::ActorCall {
                detail: error.to_string(),
            })?;
        match request {
            SystemRequest::SystemStatusQuery(query) => self.status_reply(query).await,
            other => Ok(SystemReply::SystemRequestUnimplemented(
                SystemRequestUnimplemented {
                    operation: other.operation_kind(),
                    reason: SystemUnimplementedReason::NotBuiltYet,
                },
            )),
        }
    }

    async fn status_reply(&self, query: SystemStatusQuery) -> Result<SystemReply> {
        let state = self
            .system
            .ask(ReadSystemState::expecting_at_least(0))
            .await
            .map_err(|error| Error::ActorCall {
                detail: error.to_string(),
            })?;
        Ok(SystemReply::SystemStatus(SystemStatus {
            backend: query.backend,
            health: Self::health(&state),
            readiness: Self::readiness(&state),
        }))
    }

    fn health(state: &SystemState) -> SystemHealth {
        match state.backend {
            SystemBackend::Niri => SystemHealth::Running,
        }
    }

    fn readiness(state: &SystemState) -> SystemReadiness {
        match state.backend {
            SystemBackend::Niri => SystemReadiness::Ready,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SystemCommandLine {
    arguments: Vec<OsString>,
}

impl SystemCommandLine {
    pub fn from_environment() -> Self {
        Self::from_arguments(std::env::args_os().skip(1))
    }

    pub fn from_arguments<I, S>(arguments: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<OsString>,
    {
        Self {
            arguments: arguments.into_iter().map(Into::into).collect(),
        }
    }

    pub fn daemon(&self) -> Result<SystemDaemon> {
        let socket = self.arguments.first().ok_or(Error::MissingSocket)?;
        if let Some(extra) = self.arguments.get(1) {
            return Err(Error::UnexpectedArgument {
                got: extra.to_string_lossy().to_string(),
            });
        }
        Ok(SystemDaemon::from_socket(PathBuf::from(socket)))
    }

    pub fn run(&self) -> Result<()> {
        self.daemon()?.run()
    }
}

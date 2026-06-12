//! System's daemon hooks — the only daemon code system hand-writes.
//!
//! The uniform daemon skeleton (argv parsing, async task-backed multi-listener
//! binding, request gating, peer credentials, lifecycle, and the `ExitReport`
//! entry) is emitted into `src/schema/daemon.rs` by schema-rust-next's daemon
//! emitter. System adopts the `component_decoded` working tier: the ordinary
//! `system.sock` keeps speaking the `signal-system` contract wire (the
//! component owns the per-connection `SystemFrame` decode), and the existing
//! kameo actors (`SystemSupervisor`, `SupervisionPhase`) stay the engine.
//!
//! The owner-only meta listener carries the engine-management supervision
//! protocol (the second socket the manager binds): `handle_meta_connection`
//! decodes a supervision `Frame` and drives `SupervisionPhase`.

use kameo::actor::ActorRef;
use meta_signal_system::{
    MetaSystemFrame, MetaSystemFrameBody, MetaSystemReply, MetaSystemRequest, RequestUnimplemented,
    UnimplementedReason,
};
use tokio::io::AsyncWriteExt;
use tokio::sync::OnceCell;
use triad_runtime::{
    AcceptedConnection, FrameBody as LengthPrefixedFrameBody, FrameError, LengthPrefixedCodec,
};

use signal_frame::{ExchangeIdentifier, NonEmpty, Reply, SubReply};
use signal_system::{
    SystemBackend, SystemFrame, SystemFrameBody as FrameBody, SystemReply, SystemRequest,
};

use crate::error::{Error, Result};
use crate::schema::daemon::ComponentDaemon;
use crate::supervision::{
    HandleSupervisionRequest, ReceivedSupervisionRequest, SupervisionPhase, SupervisionProfile,
};
use crate::{Configuration, SystemRequestHandler, SystemSupervisor};

/// The type-level selector for system's emitted daemon. It carries no runtime
/// data — it is the marker the emitted `DaemonCommand<SystemDaemon>` and the
/// generated runtime dispatch on, selecting system's `Configuration` /
/// `Engine` / `Error` types through the `ComponentDaemon` associated types.
#[derive(Debug)]
pub struct SystemDaemon;

/// System's daemon-facing engine: the lazily-started root actors that own all
/// mutable system state. The `component_decoded` runtime shares this engine as
/// `&Self::Engine`; the kameo mailboxes serialise each exchange, so no
/// component-internal lock is required. The actors start on first connection so
/// `build_runtime` stays synchronous and they spawn inside the daemon's tokio
/// runtime.
#[derive(Debug)]
pub struct SystemEngine {
    backend: SystemBackend,
    profile: SupervisionProfile,
    supervisor: OnceCell<ActorRef<SystemSupervisor>>,
    supervision: OnceCell<ActorRef<SupervisionPhase>>,
}

impl SystemEngine {
    pub fn from_configuration(configuration: &Configuration) -> Self {
        Self {
            backend: configuration.backend(),
            profile: SupervisionProfile::system(),
            supervisor: OnceCell::new(),
            supervision: OnceCell::new(),
        }
    }

    async fn supervisor(&self) -> &ActorRef<SystemSupervisor> {
        self.supervisor
            .get_or_init(|| SystemSupervisor::start(self.backend))
            .await
    }

    async fn supervision(&self) -> &ActorRef<SupervisionPhase> {
        self.supervision
            .get_or_init(|| SupervisionPhase::start(self.profile.clone()))
            .await
    }

    /// Serve one ordinary working connection: decode a `signal-system`
    /// `SystemFrame` request off the length-prefixed envelope, drive the
    /// request through the supervisor, and write the typed reply frame back.
    async fn handle_working_connection(&self, connection: &mut AcceptedConnection) -> Result<()> {
        let body = LengthPrefixedCodec::default()
            .read_body_async(connection.stream_mut())
            .await?
            .into_bytes();
        let received = ReceivedSystemRequest::decode(&body)?;
        let reply = SystemRequestHandler::new(self.supervisor().await.clone())
            .reply_for_request(received.request)
            .await?;
        WorkingSystemReply::new(received.exchange, reply)
            .write(connection.stream_mut())
            .await
    }

    /// Serve one owner-only meta connection. The canonical meta contract is
    /// `meta-signal-system`; the older engine-management supervision protocol
    /// still falls through here while the component manager carries both
    /// surfaces during the daemon-shell migration.
    async fn handle_meta_connection(&self, connection: &mut AcceptedConnection) -> Result<()> {
        let body = LengthPrefixedCodec::default()
            .read_body_async(connection.stream_mut())
            .await?
            .into_bytes();
        match ReceivedMetaSystemRequest::decode(&body) {
            Ok(received) => {
                let reply = MetaSystemReply::RequestUnimplemented(RequestUnimplemented {
                    operation: received.request.kind(),
                    reason: UnimplementedReason::ComponentPaused,
                });
                return WorkingMetaSystemReply::new(received.exchange, reply)
                    .write(connection.stream_mut())
                    .await;
            }
            Err(MetaSystemDecode::NotMeta) => {}
            Err(MetaSystemDecode::UnexpectedFrame(got)) => {
                return Err(Error::UnexpectedSignalFrame { got });
            }
        }
        let received = ReceivedSupervisionRequest::decode(&body)?;
        let reply = self
            .supervision()
            .await
            .ask(HandleSupervisionRequest {
                request: received.request,
            })
            .await
            .map_err(|error| Error::ActorCall {
                detail: error.to_string(),
            })?;
        WorkingSupervisionReply::new(received.exchange, reply.reply)
            .write(connection.stream_mut())
            .await
    }
}

impl ComponentDaemon for SystemDaemon {
    type Configuration = Configuration;
    type ConfigurationError = Error;
    type Engine = SystemEngine;
    type Error = Error;

    const PROCESS_NAME: &'static str = "system-daemon";

    fn load_configuration(
        path: &std::path::Path,
    ) -> std::result::Result<Self::Configuration, Self::ConfigurationError> {
        Configuration::from_binary_path(path)
    }

    fn build_runtime(
        configuration: &Self::Configuration,
    ) -> std::result::Result<Self::Engine, Self::Error> {
        Ok(SystemEngine::from_configuration(configuration))
    }

    async fn handle_working_connection(
        engine: &Self::Engine,
        mut connection: AcceptedConnection,
    ) -> Result<()> {
        engine.handle_working_connection(&mut connection).await
    }

    async fn handle_meta_connection(
        engine: &Self::Engine,
        mut connection: AcceptedConnection,
    ) -> Result<()> {
        engine.handle_meta_connection(&mut connection).await
    }
}

/// One decoded ordinary system request plus its exchange identifier.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReceivedSystemRequest {
    exchange: ExchangeIdentifier,
    request: SystemRequest,
}

impl ReceivedSystemRequest {
    pub fn decode(body: &[u8]) -> Result<Self> {
        match SystemFrame::decode(body)?.into_body() {
            FrameBody::Request { exchange, request } => {
                let (request, tail) = request.payloads.into_head_and_tail();
                if !tail.is_empty() {
                    return Err(Error::UnexpectedSignalFrame {
                        got: format!("expected one system payload, got {}", tail.len() + 1),
                    });
                }
                Ok(Self { exchange, request })
            }
            other => Err(Error::UnexpectedSignalFrame {
                got: format!("{other:?}"),
            }),
        }
    }

    pub fn exchange(&self) -> ExchangeIdentifier {
        self.exchange
    }

    pub fn request(&self) -> &SystemRequest {
        &self.request
    }
}

/// One ordinary system reply, framed and written back to the caller.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkingSystemReply {
    exchange: ExchangeIdentifier,
    reply: SystemReply,
}

impl WorkingSystemReply {
    pub fn new(exchange: ExchangeIdentifier, reply: SystemReply) -> Self {
        Self { exchange, reply }
    }

    async fn write(self, stream: &mut tokio::net::UnixStream) -> Result<()> {
        let frame = SystemFrame::new(FrameBody::Reply {
            exchange: self.exchange,
            reply: Reply::committed(NonEmpty::single(SubReply::Ok(self.reply))),
        });
        LengthPrefixedCodec::default()
            .write_body_async(stream, &LengthPrefixedFrameBody::new(frame.encode()?))
            .await?;
        stream.flush().await.map_err(FrameError::from)?;
        Ok(())
    }
}

/// One decoded meta-system request plus its exchange identifier.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReceivedMetaSystemRequest {
    exchange: ExchangeIdentifier,
    request: MetaSystemRequest,
}

impl ReceivedMetaSystemRequest {
    pub fn decode(body: &[u8]) -> std::result::Result<Self, MetaSystemDecode> {
        let frame = MetaSystemFrame::decode(body).map_err(|_| MetaSystemDecode::NotMeta)?;
        match frame.into_body() {
            MetaSystemFrameBody::Request { exchange, request } => {
                let (request, tail) = request.payloads.into_head_and_tail();
                if !tail.is_empty() {
                    return Err(MetaSystemDecode::UnexpectedFrame(format!(
                        "expected one meta-system payload, got {}",
                        tail.len() + 1
                    )));
                }
                Ok(Self { exchange, request })
            }
            other => Err(MetaSystemDecode::UnexpectedFrame(format!("{other:?}"))),
        }
    }

    pub fn exchange(&self) -> ExchangeIdentifier {
        self.exchange
    }

    pub fn request(&self) -> &MetaSystemRequest {
        &self.request
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MetaSystemDecode {
    NotMeta,
    UnexpectedFrame(String),
}

/// One meta-system reply, framed and written back to the owner client.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkingMetaSystemReply {
    exchange: ExchangeIdentifier,
    reply: MetaSystemReply,
}

impl WorkingMetaSystemReply {
    pub fn new(exchange: ExchangeIdentifier, reply: MetaSystemReply) -> Self {
        Self { exchange, reply }
    }

    async fn write(self, stream: &mut tokio::net::UnixStream) -> Result<()> {
        let frame = MetaSystemFrame::new(MetaSystemFrameBody::Reply {
            exchange: self.exchange,
            reply: Reply::committed(NonEmpty::single(SubReply::Ok(self.reply))),
        });
        LengthPrefixedCodec::default()
            .write_body_async(stream, &LengthPrefixedFrameBody::new(frame.encode()?))
            .await?;
        stream.flush().await.map_err(FrameError::from)?;
        Ok(())
    }
}

/// One supervision reply, framed and written back to the manager.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkingSupervisionReply {
    exchange: ExchangeIdentifier,
    reply: signal_persona::Reply,
}

impl WorkingSupervisionReply {
    pub fn new(exchange: ExchangeIdentifier, reply: signal_persona::Reply) -> Self {
        Self { exchange, reply }
    }

    async fn write(self, stream: &mut tokio::net::UnixStream) -> Result<()> {
        let frame = signal_persona::Frame::new(signal_persona::FrameBody::Reply {
            exchange: self.exchange,
            reply: Reply::committed(NonEmpty::single(SubReply::Ok(self.reply))),
        });
        LengthPrefixedCodec::default()
            .write_body_async(stream, &LengthPrefixedFrameBody::new(frame.encode()?))
            .await?;
        stream.flush().await.map_err(FrameError::from)?;
        Ok(())
    }
}

//! End-to-end tests for the schema-emitted `system-daemon`.
//!
//! The daemon adopts the `component_decoded` working tier: the ordinary
//! `system.sock` speaks the `signal-system` contract `SystemFrame` wrapped in
//! the emitted length-prefixed envelope, and the owner-only meta socket carries
//! the engine-management supervision protocol. These tests launch the real
//! `system-daemon` binary from a binary rkyv `SystemDaemonConfiguration` and
//! drive both sockets.

use std::io::{Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::process::{Child, Command};
use std::thread;
use std::time::{Duration, Instant};

use signal_frame::{
    ExchangeIdentifier, ExchangeLane, LaneSequence, NonEmpty, Reply, Request, SessionEpoch,
    SubReply,
};
use signal_persona::origin::{OwnerIdentity, UnixUserIdentifier};
use signal_persona::{
    ComponentHealth, ComponentKind, ComponentName, EngineManagementProtocolVersion,
    Frame as SupervisionFrame, FrameBody as SupervisionFrameBody, Operation as SupervisionRequest,
    Presence, Query as SupervisionQuery, Reply as SupervisionReply, SocketMode, WirePath,
};
use signal_system::{
    SystemBackend, SystemDaemonConfiguration, SystemFrame, SystemFrameBody, SystemHealth,
    SystemOperationKind, SystemReadiness, SystemReply, SystemRequest, SystemRequestUnimplemented,
    SystemStatus, SystemStatusQuery, SystemTarget, SystemUnimplementedReason,
};
use system::{Configuration, FocusSubscription};
use triad_runtime::DaemonConfiguration;

const MAXIMUM_FRAME_BYTES: u64 = 1024 * 1024;

struct DaemonFixture {
    root: PathBuf,
    system_socket: PathBuf,
    supervision_socket: PathBuf,
    configuration_path: PathBuf,
}

impl DaemonFixture {
    fn new(name: &str) -> Self {
        let root = std::env::temp_dir().join(format!(
            "system-{name}-{}-{}",
            std::process::id(),
            unique_nanos()
        ));
        std::fs::create_dir_all(&root).expect("fixture root created");
        Self {
            system_socket: root.join("system.sock"),
            supervision_socket: root.join("system-supervision.sock"),
            configuration_path: root.join("system-daemon.rkyv"),
            root,
        }
    }

    fn configuration(&self) -> SystemDaemonConfiguration {
        SystemDaemonConfiguration {
            system_socket_path: WirePath::new(self.system_socket.display().to_string()),
            system_socket_mode: SocketMode::new(0o600),
            supervision_socket_path: WirePath::new(self.supervision_socket.display().to_string()),
            supervision_socket_mode: SocketMode::new(0o600),
            backend: SystemBackend::Niri,
            owner_identity: OwnerIdentity::UnixUser(UnixUserIdentifier::new(1000)),
        }
    }

    fn write_configuration(&self) {
        let bytes = self
            .configuration()
            .to_rkyv_bytes()
            .expect("encode binary system configuration");
        std::fs::write(&self.configuration_path, bytes).expect("write binary system configuration");
    }

    fn spawn(&self) -> Child {
        self.write_configuration();
        let child = Command::new(env!("CARGO_BIN_EXE_system-daemon"))
            .arg(&self.configuration_path)
            .spawn()
            .expect("system-daemon starts");
        wait_for_socket_file(&self.system_socket);
        wait_for_socket_file(&self.supervision_socket);
        child
    }

    /// Open one supervision connection, send one engine-management request, and
    /// read its reply — the emitted daemon serves one request per connection.
    fn supervision_exchange(&self, request: SupervisionRequest) -> SupervisionReply {
        let mut stream =
            UnixStream::connect(&self.supervision_socket).expect("supervision client connects");
        write_supervision_request(&mut stream, request);
        read_supervision_reply(&mut stream)
    }
}

impl Drop for DaemonFixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

#[test]
fn configuration_accepts_binary_file_argument() {
    let fixture = DaemonFixture::new("binary-configuration");
    fixture.write_configuration();

    let decoded =
        Configuration::from_binary_path(&fixture.configuration_path).expect("decode configuration");

    assert_eq!(decoded.raw(), &fixture.configuration());
    assert_eq!(decoded.socket_path(), fixture.system_socket.as_path());
    assert_eq!(
        decoded.meta_socket_path(),
        Some(fixture.supervision_socket.as_path())
    );
}

#[test]
fn daemon_binds_sockets_at_the_managed_mode() {
    let fixture = DaemonFixture::new("socket-mode");
    let mut child = fixture.spawn();

    let system_mode = socket_mode(&fixture.system_socket);
    let supervision_mode = socket_mode(&fixture.supervision_socket);
    assert_eq!(system_mode, 0o600);
    assert_eq!(supervision_mode, 0o600);

    stop_child(&mut child);
}

#[test]
fn daemon_answers_status_readiness() {
    let fixture = DaemonFixture::new("status");
    let mut child = fixture.spawn();

    let mut stream = UnixStream::connect(&fixture.system_socket).expect("client connects");
    write_system_request(
        &mut stream,
        SystemRequest::QueryStatus(SystemStatusQuery {
            backend: SystemBackend::Niri,
        }),
    );
    let reply = read_system_reply(&mut stream);

    assert_eq!(
        reply,
        SystemReply::SystemStatus(SystemStatus {
            backend: SystemBackend::Niri,
            health: SystemHealth::Running,
            readiness: SystemReadiness::Ready,
        })
    );

    stop_child(&mut child);
}

#[test]
fn daemon_returns_typed_unimplemented() {
    let fixture = DaemonFixture::new("unimplemented");
    let mut child = fixture.spawn();

    let mut stream = UnixStream::connect(&fixture.system_socket).expect("client connects");
    write_system_request(
        &mut stream,
        SystemRequest::WatchFocus(FocusSubscription {
            target: SystemTarget::niri_window(223),
        }),
    );
    let reply = read_system_reply(&mut stream);

    assert_eq!(
        reply,
        SystemReply::SystemRequestUnimplemented(SystemRequestUnimplemented {
            operation: SystemOperationKind::WatchFocus,
            reason: SystemUnimplementedReason::NotBuiltYet,
        })
    );

    stop_child(&mut child);
}

#[test]
fn daemon_answers_component_supervision_relation() {
    let fixture = DaemonFixture::new("supervision");
    let mut child = fixture.spawn();

    // The emitted daemon serves one request per accepted connection on every
    // tier, so each supervision exchange opens its own connection.
    assert!(matches!(
        fixture.supervision_exchange(SupervisionRequest::Announce(Presence {
            expected_component: ComponentName::new("system"),
            expected_kind: ComponentKind::System,
            engine_management_protocol_version: EngineManagementProtocolVersion::new(1),
        })),
        SupervisionReply::Identified(identity)
            if identity.name.as_str() == "system" && identity.kind == ComponentKind::System
    ));

    assert!(matches!(
        fixture.supervision_exchange(SupervisionRequest::Query(
            SupervisionQuery::ReadinessStatus(ComponentName::new("system"))
        )),
        SupervisionReply::Ready(_)
    ));

    assert!(matches!(
        fixture.supervision_exchange(SupervisionRequest::Query(SupervisionQuery::HealthStatus(
            ComponentName::new("system")
        ))),
        SupervisionReply::HealthReport(report) if report.health == ComponentHealth::Running
    ));

    stop_child(&mut child);
}

#[test]
fn supervision_single_payload_request_round_trips() {
    let request = Request::from_payloads(NonEmpty::single(SupervisionRequest::Query(
        SupervisionQuery::HealthStatus(ComponentName::new("system")),
    )));
    let frame = SupervisionFrame::new(SupervisionFrameBody::Request {
        exchange: test_exchange(),
        request,
    });
    let bytes = frame.encode().expect("frame encodes");
    let decoded = SupervisionFrame::decode(&bytes).expect("frame decodes");
    assert_eq!(frame.into_body(), decoded.into_body());
}

fn write_system_request(stream: &mut UnixStream, request: SystemRequest) {
    let frame = SystemFrame::new(SystemFrameBody::Request {
        exchange: test_exchange(),
        request: Request::from_payload(request),
    });
    write_envelope(stream, frame.encode().expect("system request encodes"));
}

fn read_system_reply(stream: &mut UnixStream) -> SystemReply {
    let body = read_envelope(stream);
    match SystemFrame::decode(&body)
        .expect("system reply frame decodes")
        .into_body()
    {
        SystemFrameBody::Reply { reply, .. } => match reply {
            Reply::Accepted { per_operation, .. } => match per_operation.into_head() {
                SubReply::Ok(payload) => payload,
                other => panic!("expected ok system sub-reply, got {other:?}"),
            },
            Reply::Rejected { reason } => panic!("expected accepted reply, got {reason:?}"),
        },
        other => panic!("expected system reply, got {other:?}"),
    }
}

fn write_supervision_request(stream: &mut UnixStream, request: SupervisionRequest) {
    let frame = SupervisionFrame::new(SupervisionFrameBody::Request {
        exchange: test_exchange(),
        request: Request::from_payload(request),
    });
    write_envelope(stream, frame.encode().expect("supervision request encodes"));
}

fn read_supervision_reply(stream: &mut UnixStream) -> SupervisionReply {
    let body = read_envelope(stream);
    match SupervisionFrame::decode(&body)
        .expect("supervision reply frame decodes")
        .into_body()
    {
        SupervisionFrameBody::Reply { reply, .. } => match reply {
            Reply::Accepted { per_operation, .. } => match per_operation.into_head() {
                SubReply::Ok(payload) => payload,
                other => panic!("expected ok supervision sub-reply, got {other:?}"),
            },
            Reply::Rejected { reason } => panic!("expected accepted reply, got {reason:?}"),
        },
        other => panic!("expected supervision reply, got {other:?}"),
    }
}

fn write_envelope(stream: &mut UnixStream, body: Vec<u8>) {
    let length = u32::try_from(body.len()).expect("body fits length prefix");
    assert!(
        u64::from(length) <= MAXIMUM_FRAME_BYTES,
        "body within bound"
    );
    let mut framed = Vec::with_capacity(4 + body.len());
    framed.extend_from_slice(&length.to_be_bytes());
    framed.extend_from_slice(&body);
    stream.write_all(&framed).expect("request writes");
    stream.flush().expect("request flushes");
}

fn read_envelope(stream: &mut UnixStream) -> Vec<u8> {
    let mut length_bytes = [0_u8; 4];
    stream.read_exact(&mut length_bytes).expect("length prefix");
    let length = u32::from_be_bytes(length_bytes) as usize;
    let mut body = vec![0_u8; length];
    stream.read_exact(&mut body).expect("frame body");
    body
}

fn socket_mode(path: &PathBuf) -> u32 {
    std::fs::metadata(path)
        .expect("socket metadata is readable")
        .permissions()
        .mode()
        & 0o777
}

fn test_exchange() -> ExchangeIdentifier {
    ExchangeIdentifier::new(
        SessionEpoch::new(1),
        ExchangeLane::Connector,
        LaneSequence::new(1),
    )
}

fn wait_for_socket_file(path: &PathBuf) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if path.exists() {
            return;
        }
        thread::sleep(Duration::from_millis(10));
    }
    panic!("socket {path:?} did not become ready");
}

fn stop_child(child: &mut Child) {
    let _ = child.kill();
    let _ = child.wait();
}

fn unique_nanos() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system clock after epoch")
        .as_nanos()
}

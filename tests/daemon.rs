use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::process::{Child, Command};
use std::thread;
use std::time::{Duration, Instant};

use signal_engine_management::{
    ComponentHealth, ComponentKind, ComponentName, EngineManagementProtocolVersion,
    Frame as SupervisionFrame, FrameBody as SupervisionFrameBody, Operation as SupervisionRequest,
    Presence, Query as SupervisionQuery, Reply as SupervisionReply, SocketMode as WireSocketMode,
    WirePath,
};
use signal_frame::{
    ExchangeIdentifier as FrameExchangeIdentifier, ExchangeIdentifier,
    ExchangeLane as FrameExchangeLane, ExchangeLane, LaneSequence as FrameLaneSequence,
    LaneSequence, NonEmpty, Reply, Request as FrameRequest, Request as SystemSignalRequest,
    SessionEpoch, SessionEpoch as FrameSessionEpoch, SubReply,
};
use signal_persona_origin::{OwnerIdentity, UnixUserIdentifier};
use signal_system::{
    FocusSubscription, SystemBackend, SystemDaemonConfiguration, SystemFrame, SystemFrameBody,
    SystemHealth, SystemOperationKind, SystemReadiness, SystemReply, SystemRequest,
    SystemRequestUnimplemented, SystemStatus, SystemStatusQuery, SystemTarget,
    SystemUnimplementedReason,
};
use system::{
    SocketMode, SupervisionFrameCodec, SupervisionListener, SupervisionProfile,
    SupervisionSocketMode, SystemCommandLine, SystemDaemon, SystemDaemonCommand,
    SystemDaemonConfigurationFile, SystemFrameCodec,
};

struct SocketFixture {
    root: PathBuf,
    socket: PathBuf,
}

impl SocketFixture {
    fn new(name: &str) -> Self {
        let root = std::env::temp_dir().join(format!(
            "system-{name}-{}-{}",
            std::process::id(),
            unique_nanos()
        ));
        let socket = root.join("system.sock");
        std::fs::create_dir_all(&root).expect("fixture root created");
        Self { root, socket }
    }

    fn socket(&self) -> &PathBuf {
        &self.socket
    }

    fn supervision_socket(&self) -> PathBuf {
        self.root.join("system-supervision.sock")
    }

    fn configuration_path(&self) -> PathBuf {
        self.root.join("system-daemon.rkyv")
    }

    fn configuration(&self) -> SystemDaemonConfiguration {
        SystemDaemonConfiguration {
            system_socket_path: WirePath::new(self.socket.display().to_string()),
            system_socket_mode: WireSocketMode::new(0o600),
            supervision_socket_path: WirePath::new(self.supervision_socket().display().to_string()),
            supervision_socket_mode: WireSocketMode::new(0o600),
            backend: SystemBackend::Niri,
            owner_identity: OwnerIdentity::UnixUser(UnixUserIdentifier::new(1000)),
        }
    }
}

impl Drop for SocketFixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

#[test]
fn system_daemon_applies_spawn_envelope_socket_mode() {
    let fixture = SocketFixture::new("socket-mode");
    let server = SystemDaemon::from_socket(fixture.socket())
        .with_socket_mode(SocketMode::from_octal(0o600))
        .bind()
        .expect("daemon binds before client connects");

    let mode = std::fs::metadata(server.socket())
        .expect("system socket metadata is readable")
        .permissions()
        .mode()
        & 0o777;

    assert_eq!(mode, 0o600);
}

#[test]
fn system_command_line_requires_socket_path() {
    let error = SystemCommandLine::from_arguments(std::iter::empty::<&str>())
        .daemon()
        .expect_err("missing socket is typed");

    assert_eq!(error.to_string(), "system socket path is missing");
}

#[test]
fn system_daemon_configuration_accepts_binary_file_argument() {
    let fixture = SocketFixture::new("binary-configuration");
    let configuration_path = fixture.configuration_path();
    let configuration = fixture.configuration();
    SystemDaemonConfigurationFile::new(&configuration_path)
        .write_configuration(&configuration)
        .expect("write binary system configuration");

    let decoded = SystemDaemonCommand::from_arguments([configuration_path.display().to_string()])
        .configuration()
        .expect("decode binary configuration argument");

    assert_eq!(decoded, configuration);
}

#[test]
fn system_daemon_configuration_rejects_nota_arguments() {
    let fixture = SocketFixture::new("reject-nota-configuration");
    let nota_path = fixture.root.join("system-daemon.nota");
    std::fs::write(&nota_path, "(SystemDaemonConfiguration)").expect("write nota fixture");

    let inline = SystemDaemonCommand::from_arguments(["(SystemDaemonConfiguration)"])
        .configuration()
        .expect_err("inline NOTA is rejected");
    let file = SystemDaemonCommand::from_arguments([nota_path.display().to_string()])
        .configuration()
        .expect_err(".nota file is rejected");

    assert!(matches!(inline, system::Error::Argument(_)));
    assert!(matches!(file, system::Error::Argument(_)));
}

#[test]
fn system_frame_codec_rejects_multi_payload_system_requests() {
    let request = SystemSignalRequest::from_payloads(NonEmpty::from_head_and_tail(
        SystemRequest::QueryStatus(SystemStatusQuery {
            backend: SystemBackend::Niri,
        }),
        vec![SystemRequest::QueryStatus(SystemStatusQuery {
            backend: SystemBackend::Niri,
        })],
    ));
    let frame = SystemFrame::new(SystemFrameBody::Request {
        exchange: test_exchange(),
        request,
    });
    let bytes = frame.encode_length_prefixed().expect("frame encodes");
    let mut input = bytes.as_slice();
    let error = SystemFrameCodec::default()
        .read_request(&mut input)
        .expect_err("multi-payload request is rejected");

    assert!(matches!(
        error,
        system::Error::UnexpectedSignalFrame { got }
            if got == "expected one system payload, got 2"
    ));
}

#[test]
fn system_daemon_answers_status_readiness() {
    let fixture = SocketFixture::new("status");
    let server = SystemDaemon::from_socket(fixture.socket())
        .bind()
        .expect("daemon binds before client connects");
    let socket = server.socket().clone();
    let handle = thread::spawn(move || server.serve_one());

    let mut stream = UnixStream::connect(socket).expect("client connects");
    write_request(
        &mut stream,
        SystemRequest::QueryStatus(SystemStatusQuery {
            backend: SystemBackend::Niri,
        }),
    );
    let reply = read_reply(&mut stream);
    let server_reply = handle
        .join()
        .expect("daemon thread joins")
        .expect("daemon handles one request");

    let expected = SystemReply::SystemStatus(SystemStatus {
        backend: SystemBackend::Niri,
        health: SystemHealth::Running,
        readiness: SystemReadiness::Ready,
    });
    assert_eq!(reply, expected);
    assert_eq!(server_reply, expected);
}

#[test]
fn system_daemon_answers_component_supervision_relation() {
    let fixture = SocketFixture::new("supervision");
    let supervision_socket = fixture.supervision_socket();
    let _supervision = SupervisionListener::new(
        SupervisionProfile::system(),
        supervision_socket.clone(),
        SupervisionSocketMode::from_octal(0o600),
    )
    .spawn()
    .expect("system supervision listener starts");

    let mode = std::fs::metadata(&supervision_socket)
        .expect("supervision socket metadata is readable")
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(mode, 0o600);

    let mut stream = UnixStream::connect(&supervision_socket).expect("client connects");
    let codec = SupervisionFrameCodec::new(1024 * 1024);

    write_supervision_request(
        &mut stream,
        SupervisionRequest::Announce(Presence {
            expected_component: ComponentName::new("system"),
            expected_kind: ComponentKind::System,
            engine_management_protocol_version: EngineManagementProtocolVersion::new(1),
        }),
    );
    let identity = codec.read_reply(&mut stream).expect("identity reply");
    assert!(matches!(
        identity,
        SupervisionReply::Identified(identity)
            if identity.name.as_str() == "system"
                && identity.kind == ComponentKind::System
    ));

    write_supervision_request(
        &mut stream,
        SupervisionRequest::Query(SupervisionQuery::ReadinessStatus(ComponentName::new(
            "system",
        ))),
    );
    assert!(matches!(
        codec.read_reply(&mut stream).expect("readiness reply"),
        SupervisionReply::Ready(_)
    ));

    write_supervision_request(
        &mut stream,
        SupervisionRequest::Query(SupervisionQuery::HealthStatus(ComponentName::new("system"))),
    );
    assert!(matches!(
        codec.read_reply(&mut stream).expect("health reply"),
        SupervisionReply::HealthReport(report)
            if report.health == ComponentHealth::Running
    ));
}

#[test]
fn system_daemon_binary_entrypoint_answers_component_supervision_relation() {
    let fixture = SocketFixture::new("binary-entrypoint-supervision");
    let configuration_path = fixture.configuration_path();
    let configuration = fixture.configuration();
    let supervision_socket = fixture.supervision_socket();
    SystemDaemonConfigurationFile::new(&configuration_path)
        .write_configuration(&configuration)
        .expect("write binary system configuration");

    let mut child = Command::new(env!("CARGO_BIN_EXE_system-daemon"))
        .arg(&configuration_path)
        .spawn()
        .expect("system-daemon starts");

    wait_for_socket_file(fixture.socket());
    wait_for_socket_file(&supervision_socket);
    let system_mode = std::fs::metadata(fixture.socket())
        .expect("system socket metadata is readable")
        .permissions()
        .mode()
        & 0o777;
    let supervision_mode = std::fs::metadata(&supervision_socket)
        .expect("supervision socket metadata is readable")
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(system_mode, 0o600);
    assert_eq!(supervision_mode, 0o600);

    let mut stream = UnixStream::connect(&supervision_socket).expect("client connects");
    let codec = SupervisionFrameCodec::new(1024 * 1024);
    write_supervision_request(
        &mut stream,
        SupervisionRequest::Announce(Presence {
            expected_component: ComponentName::new("system"),
            expected_kind: ComponentKind::System,
            engine_management_protocol_version: EngineManagementProtocolVersion::new(1),
        }),
    );
    assert!(matches!(
        codec.read_reply(&mut stream).expect("identity reply"),
        SupervisionReply::Identified(identity)
            if identity.name.as_str() == "system"
                && identity.kind == ComponentKind::System
    ));

    write_supervision_request(
        &mut stream,
        SupervisionRequest::Query(SupervisionQuery::HealthStatus(ComponentName::new("system"))),
    );
    assert!(matches!(
        codec.read_reply(&mut stream).expect("health reply"),
        SupervisionReply::HealthReport(report)
            if report.health == ComponentHealth::Running
    ));

    stop_child(&mut child);
}

#[test]
fn system_daemon_returns_typed_unimplemented() {
    let fixture = SocketFixture::new("unimplemented");
    let server = SystemDaemon::from_socket(fixture.socket())
        .bind()
        .expect("daemon binds before client connects");
    let socket = server.socket().clone();
    let handle = thread::spawn(move || server.serve_one());

    let mut stream = UnixStream::connect(socket).expect("client connects");
    write_request(
        &mut stream,
        SystemRequest::WatchFocus(FocusSubscription {
            target: SystemTarget::niri_window(223),
        }),
    );
    let reply = read_reply(&mut stream);
    let server_reply = handle
        .join()
        .expect("daemon thread joins")
        .expect("daemon handles one request");

    let expected = SystemReply::SystemRequestUnimplemented(SystemRequestUnimplemented {
        operation: SystemOperationKind::WatchFocus,
        reason: SystemUnimplementedReason::NotBuiltYet,
    });
    assert_eq!(reply, expected);
    assert_eq!(server_reply, expected);
}

fn write_request(stream: &mut UnixStream, request: SystemRequest) {
    let frame = SystemFrame::new(SystemFrameBody::Request {
        exchange: test_exchange(),
        request: SystemSignalRequest::from_payload(request),
    });
    let bytes = frame.encode_length_prefixed().expect("request encodes");
    stream.write_all(&bytes).expect("request writes");
    stream.flush().expect("request flushes");
}

fn write_supervision_request(stream: &mut UnixStream, request: SupervisionRequest) {
    let frame = SupervisionFrame::new(SupervisionFrameBody::Request {
        exchange: test_supervision_exchange(),
        request: FrameRequest::from_payload(request),
    });
    let bytes = frame
        .encode_length_prefixed()
        .expect("supervision request encodes");
    stream
        .write_all(bytes.as_slice())
        .expect("supervision request writes");
    stream.flush().expect("supervision request flushes");
}

fn read_reply(stream: &mut UnixStream) -> SystemReply {
    let frame = SystemFrameCodec::default()
        .read_frame(stream)
        .expect("reply frame reads");
    match frame.into_body() {
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

fn test_exchange() -> ExchangeIdentifier {
    ExchangeIdentifier::new(
        SessionEpoch::new(1),
        ExchangeLane::Connector,
        LaneSequence::new(1),
    )
}

fn test_supervision_exchange() -> FrameExchangeIdentifier {
    FrameExchangeIdentifier::new(
        FrameSessionEpoch::new(1),
        FrameExchangeLane::Connector,
        FrameLaneSequence::new(1),
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

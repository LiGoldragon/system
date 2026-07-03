use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::process::Command;
use std::thread;
use std::time::{SystemTime, UNIX_EPOCH};

use meta_signal_system::{
    MetaSystemFrame, MetaSystemFrameBody, MetaSystemReply, MetaSystemRequest, RequestUnimplemented,
    UnimplementedReason,
};
use nota::NotaEncode;
use signal_frame::{NonEmpty, Reply, SubReply};
use signal_persona::{OwnerIdentity, UnixUserIdentifier};
use signal_system::{
    SocketMode, SystemBackend, SystemDaemonConfiguration, SystemFrame, SystemFrameBody,
    SystemHealth, SystemReadiness, SystemReply, SystemRequest, SystemStatus, SystemStatusQuery,
    WirePath,
};
use triad_runtime::{FrameBody as RuntimeFrameBody, LengthPrefixedCodec};

#[derive(Debug)]
struct CliSocketFixture {
    root: PathBuf,
}

impl CliSocketFixture {
    fn new(name: &str) -> Self {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock is after Unix epoch")
            .as_nanos();
        let root =
            std::env::temp_dir().join(format!("system-cli-{name}-{}-{now}", std::process::id()));
        std::fs::create_dir_all(&root).expect("create system cli fixture directory");
        Self { root }
    }

    fn socket(&self) -> PathBuf {
        self.root.join("system.sock")
    }

    fn configuration(&self) -> SystemDaemonConfiguration {
        SystemDaemonConfiguration {
            system_socket_path: WirePath::new(self.root.join("system.sock").display().to_string()),
            system_socket_mode: SocketMode::new(0o600),
            supervision_socket_path: WirePath::new(
                self.root.join("meta-system.sock").display().to_string(),
            ),
            supervision_socket_mode: SocketMode::new(0o600),
            backend: SystemBackend::Niri,
            owner_identity: OwnerIdentity::UnixUser(UnixUserIdentifier::new(1000)),
        }
    }
}

impl Drop for CliSocketFixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

#[test]
fn system_cli_reaches_working_socket_and_prints_typed_reply() {
    let fixture = CliSocketFixture::new("working");
    let listener = UnixListener::bind(fixture.socket()).expect("fake system socket binds");
    let server = thread::spawn(move || {
        let (mut stream, _address) = listener.accept().expect("system cli connects");
        let (exchange, request) = SystemCliServer::read_request(&mut stream);
        assert_eq!(
            request,
            SystemRequest::QueryStatus(SystemStatusQuery {
                backend: SystemBackend::Niri,
            })
        );
        SystemCliServer::write_reply(
            &mut stream,
            exchange,
            SystemReply::SystemStatus(SystemStatus {
                backend: SystemBackend::Niri,
                health: SystemHealth::Running,
                readiness: SystemReadiness::Ready,
            }),
        );
    });

    let request = SystemRequest::QueryStatus(SystemStatusQuery {
        backend: SystemBackend::Niri,
    })
    .to_nota();
    let output = Command::new(env!("CARGO_BIN_EXE_system"))
        .env("SYSTEM_SOCKET", fixture.socket())
        .arg(request)
        .output()
        .expect("run system cli");

    assert!(
        output.status.success(),
        "system cli failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).expect("system cli stdout is utf8");
    assert!(
        stdout.contains("SystemStatus"),
        "unexpected stdout: {stdout}"
    );
    assert!(stdout.contains("Running"), "unexpected stdout: {stdout}");
    server.join().expect("fake system server exits");
}

#[test]
fn meta_system_cli_reaches_policy_socket_and_prints_typed_reply() {
    let fixture = CliSocketFixture::new("meta");
    let configuration = fixture.configuration();
    let listener = UnixListener::bind(fixture.socket()).expect("fake meta-system socket binds");
    let expected = configuration.clone();
    let server = thread::spawn(move || {
        let (mut stream, _address) = listener.accept().expect("meta-system cli connects");
        let (exchange, request) = MetaSystemCliServer::read_request(&mut stream);
        assert_eq!(request, MetaSystemRequest::Configure(expected));
        MetaSystemCliServer::write_reply(
            &mut stream,
            exchange,
            MetaSystemReply::RequestUnimplemented(RequestUnimplemented {
                operation: meta_signal_system::OperationKind::Configure,
                reason: UnimplementedReason::ComponentPaused,
            }),
        );
    });

    let request = MetaSystemRequest::Configure(configuration).to_nota();
    let output = Command::new(env!("CARGO_BIN_EXE_meta-system"))
        .env("SYSTEM_META_SOCKET", fixture.socket())
        .arg(request)
        .output()
        .expect("run meta-system cli");

    assert!(
        output.status.success(),
        "meta-system cli failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).expect("meta-system cli stdout is utf8");
    assert!(
        stdout.contains("RequestUnimplemented"),
        "unexpected stdout: {stdout}"
    );
    assert!(
        stdout.contains("ComponentPaused"),
        "unexpected stdout: {stdout}"
    );
    server.join().expect("fake meta-system server exits");
}

#[derive(Debug)]
struct SystemCliServer;

impl SystemCliServer {
    fn read_request(stream: &mut UnixStream) -> (signal_frame::ExchangeIdentifier, SystemRequest) {
        let body = RuntimeFrame::read(stream);
        match SystemFrame::decode(body.bytes())
            .expect("decode system signal frame")
            .into_body()
        {
            SystemFrameBody::Request { exchange, request } => {
                let (payload, tail) = request.payloads.into_head_and_tail();
                assert!(tail.is_empty(), "system cli should send one payload");
                (exchange, payload)
            }
            other => panic!("expected system request frame, got {other:?}"),
        }
    }

    fn write_reply(
        stream: &mut UnixStream,
        exchange: signal_frame::ExchangeIdentifier,
        reply: SystemReply,
    ) {
        let frame = SystemFrame::new(SystemFrameBody::Reply {
            exchange,
            reply: Reply::committed(NonEmpty::single(SubReply::Ok(reply))),
        });
        RuntimeFrame::write(stream, frame.encode().expect("encode system reply"));
    }
}

#[derive(Debug)]
struct MetaSystemCliServer;

impl MetaSystemCliServer {
    fn read_request(
        stream: &mut UnixStream,
    ) -> (signal_frame::ExchangeIdentifier, MetaSystemRequest) {
        let body = RuntimeFrame::read(stream);
        match MetaSystemFrame::decode(body.bytes())
            .expect("decode meta-system signal frame")
            .into_body()
        {
            MetaSystemFrameBody::Request { exchange, request } => {
                let (payload, tail) = request.payloads.into_head_and_tail();
                assert!(tail.is_empty(), "meta-system cli should send one payload");
                (exchange, payload)
            }
            other => panic!("expected meta-system request frame, got {other:?}"),
        }
    }

    fn write_reply(
        stream: &mut UnixStream,
        exchange: signal_frame::ExchangeIdentifier,
        reply: MetaSystemReply,
    ) {
        let frame = MetaSystemFrame::new(MetaSystemFrameBody::Reply {
            exchange,
            reply: Reply::committed(NonEmpty::single(SubReply::Ok(reply))),
        });
        RuntimeFrame::write(stream, frame.encode().expect("encode meta-system reply"));
    }
}

#[derive(Debug)]
struct RuntimeFrame;

impl RuntimeFrame {
    fn read(stream: &mut UnixStream) -> RuntimeFrameBody {
        LengthPrefixedCodec::default()
            .read_body(stream)
            .expect("read runtime frame body")
    }

    fn write(stream: &mut UnixStream, bytes: Vec<u8>) {
        LengthPrefixedCodec::default()
            .write_body(stream, &RuntimeFrameBody::new(bytes))
            .expect("write runtime frame body");
    }
}

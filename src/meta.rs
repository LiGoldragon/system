use std::io::Write;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};

use meta_signal_system::{
    MetaSystemFrame, MetaSystemFrameBody, MetaSystemReply, MetaSystemRequest,
};
use nota::{NotaEncode, NotaSource};
use signal_frame::{ExchangeIdentifier, ExchangeLane, LaneSequence, Reply, SessionEpoch, SubReply};
use triad_runtime::{ComponentCommand, FrameBody as RuntimeFrameBody, LengthPrefixedCodec};

use crate::cli_argument::NotaCommandText;
use crate::{Error, Result};

const DEFAULT_META_SYSTEM_SOCKET: &str = "/tmp/meta-system.sock";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MetaSystemEndpoint {
    socket: PathBuf,
}

impl MetaSystemEndpoint {
    pub fn new(socket: impl Into<PathBuf>) -> Self {
        Self {
            socket: socket.into(),
        }
    }

    pub fn as_path(&self) -> &Path {
        &self.socket
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MetaSystemClient {
    endpoint: MetaSystemEndpoint,
    codec: LengthPrefixedCodec,
}

impl MetaSystemClient {
    pub fn new(endpoint: MetaSystemEndpoint) -> Self {
        Self {
            endpoint,
            codec: LengthPrefixedCodec::default(),
        }
    }

    pub fn submit(&self, request: MetaSystemRequest) -> Result<MetaSystemReply> {
        let exchange = self.exchange();
        let frame = MetaSystemFrame::new(MetaSystemFrameBody::Request {
            exchange,
            request: signal_frame::Request::from_payload(request),
        });
        let mut stream = UnixStream::connect(self.endpoint.as_path())?;
        self.codec
            .write_body(&mut stream, &RuntimeFrameBody::new(frame.encode()?))?;
        let body = self.codec.read_body(&mut stream)?;
        self.reply_from_frame(MetaSystemFrame::decode(body.bytes())?)
    }

    fn exchange(&self) -> ExchangeIdentifier {
        let _endpoint = &self.endpoint;
        ExchangeIdentifier::new(
            SessionEpoch::new(0),
            ExchangeLane::Connector,
            LaneSequence::first(),
        )
    }

    fn reply_from_frame(&self, frame: MetaSystemFrame) -> Result<MetaSystemReply> {
        match frame.into_body() {
            MetaSystemFrameBody::Reply { reply, .. } => self.reply_output(reply),
            other => Err(Error::UnexpectedSignalFrame {
                got: format!("{other:?}"),
            }),
        }
    }

    fn reply_output(&self, reply: Reply<MetaSystemReply>) -> Result<MetaSystemReply> {
        let _endpoint = &self.endpoint;
        match reply {
            Reply::Accepted { per_operation, .. } => match per_operation.into_head() {
                SubReply::Ok(payload) => Ok(payload),
                other => Err(Error::UnexpectedSignalFrame {
                    got: format!("{other:?}"),
                }),
            },
            Reply::Rejected { reason } => Err(Error::UnexpectedSignalFrame {
                got: reason.to_string(),
            }),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MetaSystemCommandLine {
    command: ComponentCommand,
    environment: MetaSystemCommandEnvironment,
}

impl MetaSystemCommandLine {
    pub fn from_env() -> Self {
        Self {
            command: ComponentCommand::from_environment(),
            environment: MetaSystemCommandEnvironment::from_process(),
        }
    }

    pub fn from_arguments<Arguments, Argument>(arguments: Arguments) -> Self
    where
        Arguments: IntoIterator<Item = Argument>,
        Argument: Into<String>,
    {
        Self::from_arguments_with_environment(
            arguments,
            MetaSystemCommandEnvironment::from_process(),
        )
    }

    pub fn from_arguments_with_environment<Arguments, Argument>(
        arguments: Arguments,
        environment: MetaSystemCommandEnvironment,
    ) -> Self
    where
        Arguments: IntoIterator<Item = Argument>,
        Argument: Into<String>,
    {
        Self {
            command: ComponentCommand::from_arguments(arguments),
            environment,
        }
    }

    pub fn run(self, mut output: impl Write) -> Result<()> {
        let request = MetaSystemRequestText::from_command(self.command)?.into_request()?;
        let reply = MetaSystemClient::new(self.environment.endpoint()).submit(request)?;
        writeln!(output, "{}", reply.to_nota())?;
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MetaSystemCommandEnvironment {
    socket: String,
}

impl MetaSystemCommandEnvironment {
    pub fn new(socket: impl Into<String>) -> Self {
        Self {
            socket: socket.into(),
        }
    }

    pub fn from_process() -> Self {
        Self::new(
            std::env::var("SYSTEM_META_SOCKET").unwrap_or(DEFAULT_META_SYSTEM_SOCKET.to_string()),
        )
    }

    pub fn endpoint(&self) -> MetaSystemEndpoint {
        MetaSystemEndpoint::new(&self.socket)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct MetaSystemRequestText {
    text: NotaCommandText,
}

impl MetaSystemRequestText {
    fn from_command(command: ComponentCommand) -> Result<Self> {
        Ok(Self {
            text: NotaCommandText::from_command(command)?,
        })
    }

    fn into_request(self) -> Result<MetaSystemRequest> {
        Ok(NotaSource::new(self.text.as_str()).parse::<MetaSystemRequest>()?)
    }
}

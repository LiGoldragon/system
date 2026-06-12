use std::io::Write;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};

use nota_next::{NotaEncode, NotaSource};
use signal_frame::{ExchangeIdentifier, ExchangeLane, LaneSequence, Reply, SessionEpoch, SubReply};
use signal_system::{SystemFrame, SystemFrameBody, SystemReply, SystemRequest};
use triad_runtime::{ComponentCommand, FrameBody as RuntimeFrameBody, LengthPrefixedCodec};

use crate::cli_argument::NotaCommandText;
use crate::{Error, Result};

const DEFAULT_SYSTEM_SOCKET: &str = "/tmp/system.sock";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SystemEndpoint {
    socket: PathBuf,
}

impl SystemEndpoint {
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
pub struct SystemClient {
    endpoint: SystemEndpoint,
    codec: LengthPrefixedCodec,
}

impl SystemClient {
    pub fn new(endpoint: SystemEndpoint) -> Self {
        Self {
            endpoint,
            codec: LengthPrefixedCodec::default(),
        }
    }

    pub fn submit(&self, request: SystemRequest) -> Result<SystemReply> {
        let exchange = self.exchange();
        let frame = SystemFrame::new(SystemFrameBody::Request {
            exchange,
            request: signal_frame::Request::from_payload(request),
        });
        let mut stream = UnixStream::connect(self.endpoint.as_path())?;
        self.codec
            .write_body(&mut stream, &RuntimeFrameBody::new(frame.encode()?))?;
        let body = self.codec.read_body(&mut stream)?;
        self.reply_from_frame(SystemFrame::decode(body.bytes())?)
    }

    fn exchange(&self) -> ExchangeIdentifier {
        let _endpoint = &self.endpoint;
        ExchangeIdentifier::new(
            SessionEpoch::new(0),
            ExchangeLane::Connector,
            LaneSequence::first(),
        )
    }

    fn reply_from_frame(&self, frame: SystemFrame) -> Result<SystemReply> {
        match frame.into_body() {
            SystemFrameBody::Reply { reply, .. } => self.reply_output(reply),
            other => Err(Error::UnexpectedSignalFrame {
                got: format!("{other:?}"),
            }),
        }
    }

    fn reply_output(&self, reply: Reply<SystemReply>) -> Result<SystemReply> {
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
pub struct SystemCommandLine {
    command: ComponentCommand,
    environment: SystemCommandEnvironment,
}

impl SystemCommandLine {
    pub fn from_env() -> Self {
        Self {
            command: ComponentCommand::from_environment(),
            environment: SystemCommandEnvironment::from_process(),
        }
    }

    pub fn from_arguments<Arguments, Argument>(arguments: Arguments) -> Self
    where
        Arguments: IntoIterator<Item = Argument>,
        Argument: Into<String>,
    {
        Self::from_arguments_with_environment(arguments, SystemCommandEnvironment::from_process())
    }

    pub fn from_arguments_with_environment<Arguments, Argument>(
        arguments: Arguments,
        environment: SystemCommandEnvironment,
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
        let request = SystemRequestText::from_command(self.command)?.into_request()?;
        let reply = SystemClient::new(self.environment.endpoint()).submit(request)?;
        writeln!(output, "{}", reply.to_nota())?;
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SystemCommandEnvironment {
    socket: String,
}

impl SystemCommandEnvironment {
    pub fn new(socket: impl Into<String>) -> Self {
        Self {
            socket: socket.into(),
        }
    }

    pub fn from_process() -> Self {
        Self::new(std::env::var("SYSTEM_SOCKET").unwrap_or(DEFAULT_SYSTEM_SOCKET.to_string()))
    }

    pub fn endpoint(&self) -> SystemEndpoint {
        SystemEndpoint::new(&self.socket)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SystemRequestText {
    text: NotaCommandText,
}

impl SystemRequestText {
    fn from_command(command: ComponentCommand) -> Result<Self> {
        Ok(Self {
            text: NotaCommandText::from_command(command)?,
        })
    }

    fn into_request(self) -> Result<SystemRequest> {
        Ok(NotaSource::new(self.text.as_str()).parse::<SystemRequest>()?)
    }
}

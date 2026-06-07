use std::ffi::OsString;
use std::io::Write;
use std::path::PathBuf;

use nota_next::{NotaEncode, NotaSource};
use signal_system::{
    SystemHealth, SystemReadiness, SystemReply, SystemRequest, SystemRequestUnimplemented,
    SystemStatus, SystemUnimplementedReason,
};

use crate::NiriFocusSource;
use crate::error::{Error, Result};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandLine {
    arguments: Vec<OsString>,
}

impl CommandLine {
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

    pub fn run(&self, source: &NiriFocusSource, output: impl Write) -> Result<()> {
        SystemCommand::new(self.decode_request()?).run(source, output)
    }

    pub fn decode_request(&self) -> Result<SystemRequest> {
        let Some(first) = self.arguments.first() else {
            return Err(Error::MissingInput);
        };
        self.require_single_argument()?;

        if CommandLineArgument::new(first).starts_inline_record() {
            let Some(text) = first.to_str() else {
                return Err(Error::InvalidInlineNotaArgument {
                    got: format!("{first:?}"),
                });
            };
            SystemRequestText::new(text).decode()
        } else {
            InputFile::from_path(PathBuf::from(first)).decode()
        }
    }

    fn require_single_argument(&self) -> Result<()> {
        if let Some(argument) = self.arguments.get(1) {
            return Err(Error::UnexpectedArgument {
                got: argument.to_string_lossy().to_string(),
            });
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SystemCommand {
    request: SystemRequest,
}

impl SystemCommand {
    pub fn new(request: SystemRequest) -> Self {
        Self { request }
    }

    pub fn run(self, source: &NiriFocusSource, mut output: impl Write) -> Result<()> {
        match self.request {
            SystemRequest::QueryFocus(command) => NotaLine::new(SystemReply::QueryFocusReply(
                source.observe(command.target)?,
            ))
            .write(&mut output),
            SystemRequest::WatchFocus(command) => source.subscribe(command.target, output),
            SystemRequest::QueryStatus(query) => {
                NotaLine::new(SystemReply::SystemStatus(SystemStatus {
                    backend: query.backend,
                    health: SystemHealth::Running,
                    readiness: SystemReadiness::Ready,
                }))
                .write(&mut output)
            }
            other => NotaLine::new(SystemReply::SystemRequestUnimplemented(
                SystemRequestUnimplemented {
                    operation: other.operation_kind(),
                    reason: SystemUnimplementedReason::NotBuiltYet,
                },
            ))
            .write(&mut output),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotaLine {
    reply: SystemReply,
}

impl NotaLine {
    pub fn new(reply: SystemReply) -> Self {
        Self { reply }
    }

    pub fn text(&self) -> String {
        self.reply.to_nota()
    }

    pub fn write(&self, mut output: impl Write) -> Result<()> {
        writeln!(output, "{}", self.text())?;
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct CommandLineArgument<'a> {
    argument: &'a OsString,
}

impl<'a> CommandLineArgument<'a> {
    fn new(argument: &'a OsString) -> Self {
        Self { argument }
    }

    fn starts_inline_record(self) -> bool {
        self.argument
            .as_encoded_bytes()
            .first()
            .is_some_and(|byte| *byte == b'(')
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SystemRequestText<'a> {
    text: &'a str,
}

impl<'a> SystemRequestText<'a> {
    fn new(text: &'a str) -> Self {
        Self { text }
    }

    fn decode(&self) -> Result<SystemRequest> {
        NotaSource::new(self.text).parse().map_err(Into::into)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct InputFile {
    path: PathBuf,
}

impl InputFile {
    fn from_path(path: PathBuf) -> Self {
        Self { path }
    }

    fn decode(&self) -> Result<SystemRequest> {
        let text = std::fs::read_to_string(&self.path).map_err(|source| Error::InputFileRead {
            path: self.path.clone(),
            source,
        })?;
        SystemRequestText::new(&text).decode()
    }
}

use std::collections::HashMap;
use std::io::{BufRead, Write};
use std::process::{Command, Stdio};

use nota_codec::{Encoder, NotaEncode};
use serde::Deserialize;

use crate::error::{Error, Result};
use crate::niri_focus::ApplyNiriEvent;
use crate::{FocusObservation, NiriWindowId, SystemEvent, SystemTarget};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NiriFocusSource {
    command: String,
}

impl NiriFocusSource {
    pub fn from_environment() -> Self {
        let command = std::env::var("PERSONA_NIRI_BIN").unwrap_or_else(|_| "niri".to_string());
        Self { command }
    }

    pub fn with_command(command: impl Into<String>) -> Self {
        Self {
            command: command.into(),
        }
    }

    pub fn observe(&self, target: SystemTarget) -> Result<FocusObservation> {
        let id = self.require_niri_window(target)?;
        let output = Command::new(&self.command)
            .args(["msg", "--json", "windows"])
            .output()?;
        if !output.status.success() {
            return Err(Error::NiriCommandFailed {
                detail: String::from_utf8_lossy(&output.stderr).to_string(),
            });
        }
        let windows = NiriWindows::from_json_slice(&output.stdout)?;
        windows.observe(target, id)
    }

    pub fn subscribe(&self, target: SystemTarget, mut output: impl Write) -> Result<()> {
        let id = self.require_niri_window(target)?;
        let mut tracker = FocusTracker::new(target, id);
        let windows = self.current_windows()?;
        let initial_window = windows.window(id).ok_or(Error::TargetNotFound { target })?;
        let initial = tracker.accept_window(initial_window);
        ObservationLine::new(initial).write(&mut output)?;
        output.flush()?;
        let runtime = tokio::runtime::Runtime::new()?;
        let focus = runtime.block_on(FocusTracker::start(tracker));

        let mut process = Command::new(&self.command)
            .args(["msg", "--json", "event-stream"])
            .stdout(Stdio::piped())
            .spawn()?;
        let stdout = process
            .stdout
            .take()
            .ok_or_else(|| Error::NiriCommandFailed {
                detail: "niri event-stream did not expose stdout".to_string(),
            })?;
        for line in std::io::BufReader::new(stdout).lines() {
            let line = line?;
            let event = NiriEvent::from_json_str(&line)?;
            let observations = runtime
                .block_on(focus.ask(ApplyNiriEvent { event }).send())
                .map_err(|error| Error::ActorCall {
                    detail: error.to_string(),
                })?;
            for observation in observations {
                ObservationLine::new(observation).write(&mut output)?;
                output.flush()?;
            }
        }
        runtime.block_on(FocusTracker::stop(focus))?;
        Ok(())
    }

    fn current_windows(&self) -> Result<NiriWindows> {
        let output = Command::new(&self.command)
            .args(["msg", "--json", "windows"])
            .output()?;
        if !output.status.success() {
            return Err(Error::NiriCommandFailed {
                detail: String::from_utf8_lossy(&output.stderr).to_string(),
            });
        }
        NiriWindows::from_json_slice(&output.stdout)
    }

    fn require_niri_window(&self, target: SystemTarget) -> Result<NiriWindowId> {
        target.niri_window_id().ok_or(Error::UnsupportedBackend {
            backend: format!("{target:?}"),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NiriWindows {
    windows: Vec<NiriWindowSnapshot>,
}

impl NiriWindows {
    pub fn from_json_slice(bytes: &[u8]) -> Result<Self> {
        Ok(Self {
            windows: serde_json::from_slice(bytes)?,
        })
    }

    pub fn observe(&self, target: SystemTarget, id: NiriWindowId) -> Result<FocusObservation> {
        let window = self
            .windows
            .iter()
            .find(|window| window.id == id.value())
            .ok_or(Error::TargetNotFound { target })?;
        Ok(window.observation(target))
    }

    pub fn window(&self, id: NiriWindowId) -> Option<&NiriWindowSnapshot> {
        self.windows.iter().find(|window| window.id == id.value())
    }
}

#[derive(Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct NiriWindowSnapshot {
    id: u64,
    #[allow(dead_code)]
    title: Option<String>,
    #[allow(dead_code)]
    app_id: Option<String>,
    #[allow(dead_code)]
    pid: Option<u32>,
    #[allow(dead_code)]
    workspace_id: Option<u64>,
    is_focused: bool,
    focus_timestamp: Option<NiriTimestamp>,
}

impl NiriWindowSnapshot {
    pub fn observation(&self, target: SystemTarget) -> FocusObservation {
        FocusObservation::new(target, self.is_focused, self.generation())
    }

    fn generation(&self) -> u64 {
        self.focus_timestamp
            .as_ref()
            .map(NiriTimestamp::generation)
            .unwrap_or(self.id)
    }
}

#[derive(Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct NiriTimestamp {
    secs: u64,
    nanos: u32,
}

impl NiriTimestamp {
    pub fn generation(&self) -> u64 {
        self.secs
            .saturating_mul(1_000_000_000)
            .saturating_add(u64::from(self.nanos))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NiriEvent {
    WindowsChanged {
        windows: Vec<NiriWindowSnapshot>,
    },
    WindowOpenedOrChanged {
        window: NiriWindowSnapshot,
    },
    WindowClosed {
        id: u64,
    },
    WorkspacesChanged {
        workspaces: Vec<NiriWorkspaceSnapshot>,
    },
    WorkspaceActiveWindowChanged {
        workspace_id: u64,
        active_window_id: Option<u64>,
    },
    WindowFocusChanged {
        id: u64,
    },
    WindowFocusTimestampChanged {
        id: u64,
        focus_timestamp: NiriTimestamp,
    },
    KeyboardLayoutsChanged {
        keyboard_layouts: serde_json::Value,
    },
    OverviewOpenedOrClosed {
        is_open: bool,
    },
    ConfigLoaded {
        failed: bool,
    },
    Other,
}

impl NiriEvent {
    pub fn from_json_str(text: &str) -> Result<Self> {
        let value: serde_json::Value = serde_json::from_str(text)?;
        NiriEventJson::new(value).decode()
    }

    fn windows(&self) -> Vec<&NiriWindowSnapshot> {
        match self {
            Self::WindowsChanged { windows } => windows.iter().collect(),
            Self::WindowOpenedOrChanged { window } => vec![window],
            Self::WindowClosed { .. }
            | Self::WorkspacesChanged { .. }
            | Self::WorkspaceActiveWindowChanged { .. }
            | Self::WindowFocusChanged { .. }
            | Self::WindowFocusTimestampChanged { .. }
            | Self::KeyboardLayoutsChanged { .. }
            | Self::OverviewOpenedOrClosed { .. }
            | Self::ConfigLoaded { .. }
            | Self::Other => Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct NiriEventJson {
    value: serde_json::Value,
}

impl NiriEventJson {
    fn new(value: serde_json::Value) -> Self {
        Self { value }
    }

    fn decode(self) -> Result<NiriEvent> {
        let Some(object) = self.value.as_object() else {
            return Ok(NiriEvent::Other);
        };
        let Some((kind, body)) = object.iter().next() else {
            return Ok(NiriEvent::Other);
        };
        match kind.as_str() {
            "WindowsChanged" => Ok(NiriEvent::WindowsChanged {
                windows: serde_json::from_value(body["windows"].clone())?,
            }),
            "WindowOpenedOrChanged" => Ok(NiriEvent::WindowOpenedOrChanged {
                window: serde_json::from_value(body["window"].clone())?,
            }),
            "WindowClosed" => Ok(NiriEvent::WindowClosed {
                id: serde_json::from_value(body["id"].clone())?,
            }),
            "WorkspacesChanged" => Ok(NiriEvent::WorkspacesChanged {
                workspaces: serde_json::from_value(body["workspaces"].clone())?,
            }),
            "WorkspaceActiveWindowChanged" => Ok(NiriEvent::WorkspaceActiveWindowChanged {
                workspace_id: serde_json::from_value(body["workspace_id"].clone())?,
                active_window_id: serde_json::from_value(body["active_window_id"].clone())?,
            }),
            "WindowFocusChanged" => Ok(NiriEvent::WindowFocusChanged {
                id: serde_json::from_value(body["id"].clone())?,
            }),
            "WindowFocusTimestampChanged" => Ok(NiriEvent::WindowFocusTimestampChanged {
                id: serde_json::from_value(body["id"].clone())?,
                focus_timestamp: serde_json::from_value(body["focus_timestamp"].clone())?,
            }),
            "KeyboardLayoutsChanged" => Ok(NiriEvent::KeyboardLayoutsChanged {
                keyboard_layouts: body["keyboard_layouts"].clone(),
            }),
            "OverviewOpenedOrClosed" => Ok(NiriEvent::OverviewOpenedOrClosed {
                is_open: serde_json::from_value(body["is_open"].clone())?,
            }),
            "ConfigLoaded" => Ok(NiriEvent::ConfigLoaded {
                failed: serde_json::from_value(body["failed"].clone())?,
            }),
            _ => Ok(NiriEvent::Other),
        }
    }
}

#[derive(Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct NiriWorkspaceSnapshot {
    id: u64,
    #[allow(dead_code)]
    active_window_id: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FocusTracker {
    target: SystemTarget,
    id: NiriWindowId,
    last: Option<FocusObservation>,
    generations: HashMap<u64, u64>,
    workspace_id: Option<u64>,
    synthetic_generation: u64,
    applied_event_count: u64,
    emitted_observation_count: u64,
}

impl FocusTracker {
    pub fn new(target: SystemTarget, id: NiriWindowId) -> Self {
        Self {
            target,
            id,
            last: None,
            generations: HashMap::new(),
            workspace_id: None,
            synthetic_generation: 0,
            applied_event_count: 0,
            emitted_observation_count: 0,
        }
    }

    pub fn accept(&mut self, observation: FocusObservation) {
        self.generations
            .insert(self.id.value(), observation.generation.into_u64());
        self.last = Some(observation);
    }

    pub fn accept_window(&mut self, window: &NiriWindowSnapshot) -> FocusObservation {
        self.workspace_id = window.workspace_id;
        let observation = window.observation(self.target);
        self.accept(observation);
        observation
    }

    pub fn apply_event(&mut self, event: &NiriEvent) -> Vec<FocusObservation> {
        let mut observations = Vec::new();
        for window in event.windows() {
            if window.id != self.id.value() {
                continue;
            }
            self.workspace_id = window.workspace_id;
            let observation = window.observation(self.target);
            if self.should_emit(&observation) {
                self.accept(observation);
                observations.push(observation);
            }
        }
        match event {
            NiriEvent::WorkspaceActiveWindowChanged {
                workspace_id,
                active_window_id,
            } if Some(*workspace_id) == self.workspace_id => {
                let observation = FocusObservation::new(
                    self.target,
                    active_window_id.is_some_and(|id| id == self.id.value()),
                    self.next_generation(),
                );
                if self.should_emit(&observation) {
                    self.accept(observation);
                    observations.push(observation);
                }
            }
            NiriEvent::WindowFocusChanged { id } if *id == self.id.value() => {
                let observation = FocusObservation::new(self.target, true, self.next_generation());
                if self.should_emit(&observation) {
                    self.accept(observation);
                    observations.push(observation);
                }
            }
            NiriEvent::WindowFocusTimestampChanged {
                id,
                focus_timestamp,
            } if *id == self.id.value() => {
                let observation =
                    FocusObservation::new(self.target, true, focus_timestamp.generation());
                if self.should_emit(&observation) {
                    self.accept(observation);
                    observations.push(observation);
                }
            }
            _ => {}
        }
        observations
    }

    pub(crate) fn apply_event_from_mailbox(&mut self, event: &NiriEvent) -> Vec<FocusObservation> {
        self.applied_event_count = self.applied_event_count.saturating_add(1);
        let observations = self.apply_event(event);
        self.emitted_observation_count = self
            .emitted_observation_count
            .saturating_add(observations.len() as u64);
        observations
    }

    pub(crate) fn applied_event_count(&self) -> u64 {
        self.applied_event_count
    }

    pub(crate) fn emitted_observation_count(&self) -> u64 {
        self.emitted_observation_count
    }

    fn next_generation(&mut self) -> u64 {
        self.synthetic_generation = self.synthetic_generation.saturating_add(1);
        self.last
            .map(|last| {
                last.generation
                    .into_u64()
                    .saturating_add(self.synthetic_generation)
            })
            .unwrap_or(self.synthetic_generation)
    }

    fn should_emit(&self, observation: &FocusObservation) -> bool {
        match self.last {
            Some(last) => {
                last.focused != observation.focused
                    || (last.generation != observation.generation
                        && self
                            .generations
                            .get(&self.id.value())
                            .is_none_or(|generation| {
                                *generation != observation.generation.into_u64()
                            }))
            }
            None => true,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ObservationLine {
    observation: FocusObservation,
}

impl ObservationLine {
    fn new(observation: FocusObservation) -> Self {
        Self { observation }
    }

    fn text(&self) -> Result<String> {
        let mut encoder = Encoder::new();
        let event = SystemEvent::FocusObservation(self.observation);
        event.encode(&mut encoder)?;
        Ok(encoder.into_string())
    }

    fn write(&self, mut output: impl Write) -> Result<()> {
        writeln!(output, "{}", self.text()?)?;
        Ok(())
    }
}

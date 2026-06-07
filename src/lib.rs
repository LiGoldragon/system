pub mod command;
pub mod daemon;
pub mod daemon_command;
pub mod error;
pub mod event;
pub mod niri;
pub mod niri_focus;
pub mod supervision;
pub mod target;

pub use command::CommandLine;
pub use daemon::{
    BoundSystemDaemon, SocketMode, SystemCommandLine, SystemConnection, SystemDaemon,
    SystemFrameCodec, SystemRequestHandler, SystemState, SystemSupervisor,
};
pub use daemon_command::{SystemDaemonCommand, SystemDaemonConfigurationFile};
pub use error::Error;
pub use event::FocusState;
pub use niri::{FocusTracker, NiriEvent, NiriFocusSource, NiriWindowSnapshot, NiriWindows};
pub use niri_focus::{ApplyNiriEvent, FocusStatistics, FocusStatisticsProbe, ReadFocusStatistics};
pub use signal_system::{
    FocusObservation, FocusSnapshot, FocusSubscription, FocusSubscriptionToken, NiriWindowId,
    ObservationGeneration, ObservationTargetMissing, SubscriptionAccepted, SubscriptionKind,
    SubscriptionRetracted, SystemBackend, SystemEvent, SystemHealth, SystemOperationKind,
    SystemReadiness, SystemRequest, SystemRequestUnimplemented, SystemStatus, SystemStatusQuery,
    SystemTarget, SystemUnimplementedReason, WindowClosed,
};
pub use supervision::{
    SupervisionFrameCodec, SupervisionListener, SupervisionProfile, SupervisionSocketMode,
};
pub use target::HarnessTarget;

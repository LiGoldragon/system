pub mod command;
pub mod configuration;
pub mod daemon;
pub mod error;
pub mod event;
pub mod niri;
pub mod niri_focus;
pub mod schema;
pub mod supervision;
pub mod supervisor;
pub mod target;

pub use command::CommandLine;
pub use configuration::Configuration;
pub use daemon::{
    ReceivedSystemRequest, SystemDaemon, SystemEngine, WorkingSupervisionReply, WorkingSystemReply,
};
pub use error::{Error, Result};
pub use event::FocusState;
pub use niri::{FocusTracker, NiriEvent, NiriFocusSource, NiriWindowSnapshot, NiriWindows};
pub use niri_focus::{ApplyNiriEvent, FocusStatistics, FocusStatisticsProbe, ReadFocusStatistics};
pub use schema::daemon::{ComponentDaemon, DaemonEntry};
pub use signal_system::{
    FocusObservation, FocusSnapshot, FocusSubscription, FocusSubscriptionToken, NiriWindowId,
    ObservationGeneration, ObservationTargetMissing, SubscriptionAccepted, SubscriptionKind,
    SubscriptionRetracted, SystemBackend, SystemDaemonConfiguration, SystemEvent, SystemHealth,
    SystemOperationKind, SystemReadiness, SystemRequest, SystemRequestUnimplemented, SystemStatus,
    SystemStatusQuery, SystemTarget, SystemUnimplementedReason, WindowClosed,
};
pub use supervision::{
    HandleSupervisionRequest, ReceivedSupervisionRequest, SupervisionPhase, SupervisionPhaseReply,
    SupervisionProfile,
};
pub use supervisor::{
    ReadSystemState, RecordServedSystemRequest, SystemRequestHandler, SystemState, SystemSupervisor,
};
pub use target::HarnessTarget;

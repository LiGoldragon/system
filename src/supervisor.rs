//! The system root actor (`SystemSupervisor`) and the request handler that
//! drives it. `SystemSupervisor` owns the served-request counter and the active
//! backend; it answers status/readiness for the daemon skeleton and records
//! each served request. The live Niri event-stream path attaches here on
//! unpause. The schema-emitted daemon shell shares the engine across
//! connections, and the kameo mailbox serialises every exchange.

use kameo::actor::{Actor, ActorRef, Spawn};
use kameo::error::Infallible;
use kameo::message::{Context, Message};
use signal_system::{
    SystemBackend, SystemHealth, SystemOperationKind, SystemReadiness, SystemReply, SystemRequest,
    SystemRequestUnimplemented, SystemStatus, SystemStatusQuery, SystemUnimplementedReason,
};

use crate::error::{Error, Result};

#[derive(Debug, Clone, PartialEq, Eq, kameo::Reply)]
pub struct SystemState {
    pub backend: SystemBackend,
    pub served_request_count: u64,
    pub last_operation: Option<SystemOperationKind>,
}

#[derive(Debug)]
pub struct SystemSupervisor {
    backend: SystemBackend,
    served_request_count: u64,
    last_operation: Option<SystemOperationKind>,
}

impl SystemSupervisor {
    pub fn new(backend: SystemBackend) -> Self {
        Self {
            backend,
            served_request_count: 0,
            last_operation: None,
        }
    }

    pub async fn start(backend: SystemBackend) -> ActorRef<Self> {
        let reference = Self::spawn(backend);
        reference.wait_for_startup().await;
        reference
    }

    pub async fn stop(reference: ActorRef<Self>) -> Result<()> {
        reference
            .stop_gracefully()
            .await
            .map_err(|error| Error::ActorCall {
                detail: error.to_string(),
            })?;
        reference.wait_for_shutdown().await;
        Ok(())
    }

    fn state(&self) -> SystemState {
        SystemState {
            backend: self.backend,
            served_request_count: self.served_request_count,
            last_operation: self.last_operation,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReadSystemState {
    pub minimum_served_request_count: u64,
}

impl ReadSystemState {
    pub fn expecting_at_least(minimum_served_request_count: u64) -> Self {
        Self {
            minimum_served_request_count,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RecordServedSystemRequest {
    pub operation: SystemOperationKind,
}

impl Actor for SystemSupervisor {
    type Args = SystemBackend;
    type Error = Infallible;

    async fn on_start(
        backend: Self::Args,
        _actor_reference: ActorRef<Self>,
    ) -> std::result::Result<Self, Self::Error> {
        Ok(Self::new(backend))
    }
}

impl Message<ReadSystemState> for SystemSupervisor {
    type Reply = SystemState;

    async fn handle(
        &mut self,
        message: ReadSystemState,
        _context: &mut Context<Self, Self::Reply>,
    ) -> Self::Reply {
        let _satisfied = self.served_request_count >= message.minimum_served_request_count;
        self.state()
    }
}

impl Message<RecordServedSystemRequest> for SystemSupervisor {
    type Reply = SystemState;

    async fn handle(
        &mut self,
        message: RecordServedSystemRequest,
        _context: &mut Context<Self, Self::Reply>,
    ) -> Self::Reply {
        self.last_operation = Some(message.operation);
        self.served_request_count = self.served_request_count.saturating_add(1);
        self.state()
    }
}

/// Drives one decoded `SystemRequest` through the system supervisor and returns
/// the typed `SystemReply`. The daemon skeleton answers status/readiness and
/// returns a typed `SystemRequestUnimplemented` for every unbuilt domain
/// request — never hanging or printing untyped text.
#[derive(Debug, Clone)]
pub struct SystemRequestHandler {
    system: ActorRef<SystemSupervisor>,
}

impl SystemRequestHandler {
    pub fn new(system: ActorRef<SystemSupervisor>) -> Self {
        Self { system }
    }

    pub async fn reply_for_request(&self, request: SystemRequest) -> Result<SystemReply> {
        let operation = request.operation_kind();
        let _state = self
            .system
            .ask(RecordServedSystemRequest { operation })
            .await
            .map_err(|error| Error::ActorCall {
                detail: error.to_string(),
            })?;
        match request {
            SystemRequest::QueryStatus(query) => self.status_reply(query).await,
            other => Ok(SystemReply::SystemRequestUnimplemented(
                SystemRequestUnimplemented {
                    operation: other.operation_kind(),
                    reason: SystemUnimplementedReason::NotBuiltYet,
                },
            )),
        }
    }

    async fn status_reply(&self, query: SystemStatusQuery) -> Result<SystemReply> {
        let state = self
            .system
            .ask(ReadSystemState::expecting_at_least(0))
            .await
            .map_err(|error| Error::ActorCall {
                detail: error.to_string(),
            })?;
        Ok(SystemReply::SystemStatus(SystemStatus {
            backend: query.backend,
            health: Self::health(&state),
            readiness: Self::readiness(&state),
        }))
    }

    fn health(state: &SystemState) -> SystemHealth {
        match state.backend {
            SystemBackend::Niri => SystemHealth::Running,
        }
    }

    fn readiness(state: &SystemState) -> SystemReadiness {
        match state.backend {
            SystemBackend::Niri => SystemReadiness::Ready,
        }
    }
}

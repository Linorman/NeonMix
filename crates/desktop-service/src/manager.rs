//! Nonblocking lifecycle control and immutable status publication. This mutex
//! never protects network, persistence or child-exit waits.
use crate::{Request, Result, ServiceStatus, managed::ManagedSignal};
use serde_json::{Value, json};
use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
};
use uuid::Uuid;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Kind {
    Hub,
    Sender,
    Shutdown,
}
#[derive(Clone, Copy)]
pub(crate) struct StartTicket {
    kind: Kind,
    generation: u64,
}
struct Operation {
    id: Uuid,
    kind: Kind,
    result: Option<Result<Value>>,
}
struct State {
    instance: Uuid,
    stop_generation: [u64; 2],
    shutting_down: bool,
    status: Option<ServiceStatus>,
    hub: Option<ManagedSignal>,
    sender: Option<ManagedSignal>,
    #[cfg(target_os = "linux")]
    virtual_output: Option<ManagedSignal>,
    operations: VecDeque<Operation>,
}
#[derive(Clone)]
pub(crate) struct Manager {
    state: Arc<Mutex<State>>,
}
impl Manager {
    #[cfg(target_os = "linux")]
    pub(crate) fn instance_generation(&self) -> Uuid {
        self.state
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .instance
    }
    pub(crate) fn new() -> Self {
        Self {
            state: Arc::new(Mutex::new(State {
                instance: Uuid::new_v4(),
                stop_generation: [0; 2],
                shutting_down: false,
                status: None,
                hub: None,
                sender: None,
                #[cfg(target_os = "linux")]
                virtual_output: None,
                operations: VecDeque::new(),
            })),
        }
    }
    pub(crate) fn is_shutdown(&self) -> bool {
        self.state
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .shutting_down
    }
    pub(crate) fn ticket(&self, kind: Kind) -> Result<StartTicket> {
        let state = self.state.lock().unwrap_or_else(|p| p.into_inner());
        if state.shutting_down {
            return Err("background_shutting_down".into());
        }
        Ok(StartTicket {
            kind,
            generation: state.stop_generation[usize::from(kind == Kind::Sender)],
        })
    }
    pub(crate) fn matches_instance(&self, instance: Uuid) -> bool {
        self.state
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .instance
            == instance
    }
    pub(crate) fn bound_ticket(
        &self,
        kind: Kind,
        instance: Uuid,
        generation: u64,
    ) -> Result<StartTicket> {
        let state = self.state.lock().unwrap_or_else(|p| p.into_inner());
        if state.instance != instance
            || state.shutting_down
            || state.stop_generation[usize::from(kind == Kind::Sender)] != generation
        {
            return Err("request_interrupted".into());
        }
        Ok(StartTicket { kind, generation })
    }
    pub(crate) fn install(
        &self,
        ticket: StartTicket,
        start: impl FnOnce() -> Result<ManagedSignal>,
    ) -> Result<()> {
        let mut state = self.state.lock().unwrap_or_else(|p| p.into_inner());
        if state.shutting_down
            || state.stop_generation[usize::from(ticket.kind == Kind::Sender)] != ticket.generation
        {
            return Err("request_interrupted".into());
        }
        let signal = start()?;
        match ticket.kind {
            Kind::Hub => state.hub = Some(signal),
            Kind::Sender => state.sender = Some(signal),
            Kind::Shutdown => unreachable!(),
        }
        Ok(())
    }
    pub(crate) fn publish(
        &self,
        status: ServiceStatus,
        hub: ManagedSignal,
        sender: ManagedSignal,
        #[cfg(target_os = "linux")] virtual_output: ManagedSignal,
    ) {
        let mut state = self.state.lock().unwrap_or_else(|p| p.into_inner());
        state.status = Some(status);
        state.hub = Some(hub);
        state.sender = Some(sender);
        #[cfg(target_os = "linux")]
        {
            state.virtual_output = Some(virtual_output);
        }
    }
    pub(crate) fn status(&self) -> Result<Value> {
        let state = self.state.lock().unwrap_or_else(|p| p.into_inner());
        let mut status = state.status.clone().ok_or("background_starting")?;
        if let Some(hub) = &state.hub {
            status.hub = hub.status();
        }
        if let Some(sender) = &state.sender {
            status.sender = sender.status();
        }
        if !status.sender.running {
            status.sender_options = None;
        }
        status.lifecycle = Some(crate::LifecycleView {
            version: 1,
            instance_generation: state.instance,
            hub_stop_generation: state.stop_generation[0],
            sender_stop_generation: state.stop_generation[1],
        });
        let mut value = serde_json::to_value(status).map_err(|_| "invalid_backend_response")?;
        value["lifecycle_version"] = json!(1);
        value["instance_generation"] = json!(state.instance);
        value["shutting_down"] = json!(state.shutting_down);
        Ok(value)
    }
    pub(crate) fn lookup(&self, id: Uuid, instance: Uuid) -> Result<Value> {
        let state = self.state.lock().unwrap_or_else(|p| p.into_inner());
        if state.instance != instance {
            return Err("request_interrupted".into());
        }
        let op = state
            .operations
            .iter()
            .find(|op| op.id == id)
            .ok_or("invalid_argument")?;
        Ok(operation_value(op, state.instance, state.stop_generation))
    }
    pub(crate) fn stop(
        &self,
        kind: Kind,
        shutdown: tokio::sync::mpsc::Sender<()>,
    ) -> Result<Value> {
        let mut state = self.state.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(op) = state.operations.iter().find(|op| {
            op.kind == kind
                && (op.result.is_none()
                    || (kind == Kind::Shutdown && op.result.as_ref().is_some_and(Result::is_ok)))
        }) {
            return Ok(operation_value(op, state.instance, state.stop_generation));
        }
        if state.operations.len() == 32 {
            let index = state
                .operations
                .iter()
                .position(|op| op.result.is_some())
                .ok_or("background_busy")?;
            state.operations.remove(index);
        }
        for (index, target) in [Kind::Hub, Kind::Sender].into_iter().enumerate() {
            if kind == target || kind == Kind::Shutdown {
                state.stop_generation[index] = state.stop_generation[index]
                    .checked_add(1)
                    .ok_or("quota_exceeded")?;
            }
        }
        if kind == Kind::Shutdown {
            state.shutting_down = true;
        }
        let id = Uuid::new_v4();
        let hub = (kind != Kind::Sender).then(|| state.hub.clone()).flatten();
        let sender = (kind != Kind::Hub).then(|| state.sender.clone()).flatten();
        #[cfg(target_os = "linux")]
        let virtual_output = (kind == Kind::Shutdown)
            .then(|| state.virtual_output.clone())
            .flatten();
        state.operations.push_back(Operation {
            id,
            kind,
            result: None,
        });
        let reply = operation_value(
            state.operations.back().expect("inserted operation"),
            state.instance,
            state.stop_generation,
        );
        let manager = self.clone();
        tokio::spawn(async move {
            async fn stop(signal: Option<ManagedSignal>) -> Result<Value> {
                match signal {
                    Some(signal) => signal.stop().await.and_then(|result| {
                        serde_json::to_value(result).map_err(|_| "invalid_backend_response".into())
                    }),
                    None => Ok(Value::Null),
                }
            }
            #[cfg(not(target_os = "linux"))]
            let (hub, sender) = tokio::join!(stop(hub), stop(sender));
            #[cfg(target_os = "linux")]
            let (hub, sender, virtual_output) =
                tokio::join!(stop(hub), stop(sender), stop(virtual_output));
            let result = match (hub, sender) {
                (Ok(hub), Ok(sender)) => Ok(
                    json!({"stop_result":match kind {Kind::Hub=>hub.clone(),Kind::Sender=>sender.clone(),Kind::Shutdown=>Value::Null},"hub":hub,"sender":sender,"automatic_restart":false,"event":match kind {Kind::Hub=>"hub_stopped",Kind::Sender=>"sender_user_stopped",Kind::Shutdown=>"background_stopped"}}),
                ),
                (Err(error), _) | (_, Err(error)) => Err(error),
            };
            #[cfg(target_os = "linux")]
            let result = match virtual_output {
                Ok(_) => result,
                Err(error) => Err(error),
            };
            let completed = result.is_ok();
            {
                let mut state = manager.state.lock().unwrap_or_else(|p| p.into_inner());
                if let Some(op) = state.operations.iter_mut().find(|op| op.id == id) {
                    op.result = Some(result);
                }
            }
            if kind == Kind::Shutdown && completed {
                // Reaping is already complete. Allow one short operation query,
                // then exit even if the client disappeared before the reply.
                tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                let _ = shutdown.send(()).await;
            }
        });
        Ok(reply)
    }
}
fn operation_value(op: &Operation, instance: Uuid, stop_generation: [u64; 2]) -> Value {
    let mut value = json!({"operation_id":op.id,"instance_generation":instance,"state":if op.result.is_none(){"accepted"}else{"completed"}});
    if let Some(result) = &op.result {
        match result {
            Ok(data) => {
                value["ok"] = json!(true);
                value["data"] = data.clone();
                value["data"]["lifecycle"] = json!(crate::LifecycleView {
                    version: 1,
                    instance_generation: instance,
                    hub_stop_generation: stop_generation[0],
                    sender_stop_generation: stop_generation[1],
                });
            }
            Err(error) => {
                value["ok"] = json!(false);
                value["fault"] = serde_json::to_value(
                    crate::Fault::from_error(error)
                        .unwrap_or_else(|| crate::Fault::new(crate::FaultCode::GenericFailure)),
                )
                .unwrap_or(Value::Null);
            }
        }
    }
    value
}
pub(crate) fn kind(request: &Request) -> Option<Kind> {
    match request {
        Request::LifecycleStop { request, .. } => kind(request),
        Request::HubStop => Some(Kind::Hub),
        Request::SenderStop => Some(Kind::Sender),
        Request::Shutdown => Some(Kind::Shutdown),
        _ => None,
    }
}
pub(crate) fn is_lifecycle(request: &Request) -> bool {
    kind(request).is_some()
        || matches!(
            request,
            Request::Status | Request::LifecycleOperation { .. }
        )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn completed_shutdown_is_reused_without_advancing_stop_generations_again() {
        let manager = Manager::new();
        let (shutdown, mut requested) = tokio::sync::mpsc::channel(1);
        let accepted = manager.stop(Kind::Shutdown, shutdown.clone()).unwrap();
        tokio::task::yield_now().await;
        let repeated = manager.stop(Kind::Shutdown, shutdown).unwrap();
        assert_eq!(accepted["operation_id"], repeated["operation_id"]);
        assert_eq!(repeated["state"], "completed");
        assert_eq!(manager.state.lock().unwrap().stop_generation, [1, 1]);
        assert!(manager.ticket(Kind::Hub).is_err());
        assert!(manager.ticket(Kind::Sender).is_err());
        tokio::time::timeout(std::time::Duration::from_secs(2), requested.recv())
            .await
            .unwrap()
            .unwrap();
    }
    #[tokio::test]
    async fn accepted_stop_fences_queued_start_and_duplicate_stop_reuses_the_operation() {
        let manager = Manager::new();
        let ticket = manager.ticket(Kind::Hub).unwrap();
        let (shutdown, _) = tokio::sync::mpsc::channel(1);
        let first = manager.stop(Kind::Hub, shutdown.clone()).unwrap();
        let repeated = manager.stop(Kind::Hub, shutdown).unwrap();
        assert_eq!(first["operation_id"], repeated["operation_id"]);
        assert!(
            manager
                .install(ticket, || panic!("stopped start reached spawn"))
                .is_err()
        );
        let id = serde_json::from_value(first["operation_id"].clone()).unwrap();
        assert!(
            manager.lookup(id, Uuid::new_v4()).is_err(),
            "old instance operation was adopted"
        );
        tokio::task::yield_now().await;
        let instance = serde_json::from_value(first["instance_generation"].clone()).unwrap();
        assert_eq!(manager.lookup(id, instance).unwrap()["state"], "completed");
        let ticket = manager.ticket(Kind::Hub).unwrap();
        assert_eq!(
            manager
                .install(ticket, || Err("fixture_spawn_failure".into()))
                .unwrap_err(),
            "fixture_spawn_failure"
        );
    }
}

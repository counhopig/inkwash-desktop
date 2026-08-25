//! Shared application state held by the Tauri runtime.
//!
//! Two invariants to be aware of when reading this file:
//!
//! 1. `link` is *never* held across an `recv_timeout()` - callers take a
//!    cheap [`ActiveLink::transport`] snapshot (an `Arc` clone) under the
//!    lock, release it, then wait on the snapshot - so a slow USB sync
//!    does not block BLE scans, log reads, or any other command that
//!    needs to look at the link state.
//! 2. `logs.append()` always logs to stderr AND to the on-disk file AND
//!    emits a `device-log` event - CLI mode picks up the stderr line,
//!    the GUI picks up the event, the file is the permanent record.

use std::sync::{Arc, Condvar, Mutex};
use std::time::Instant;

use tauri::AppHandle;

use crate::commands::device::DeviceCommandResult;
use crate::commands::logs::LogStore;
use crate::error::AppError;
use crate::transport::Transport;

pub struct AppState {
    pub link: Mutex<LinkState>,
    /// The single in-flight device request, used by `send_and_wait` to
    /// coalesce identical concurrent commands (e.g. the duplicate
    /// `get_status` probes the UI pages race into during the USB boot
    /// window) into one wire exchange.
    pub inflight: InflightRegistry,
    pub logs: LogStore,
    pub ctx: AppHandle,
}

impl AppState {
    pub fn new(ctx: AppHandle) -> Self {
        let logs = LogStore::new(ctx.clone());
        logs.info("app", "Inkwash Desktop started");
        Self {
            link: Mutex::new(LinkState::Disconnected),
            inflight: InflightRegistry::new(),
            logs,
            ctx,
        }
    }
}

#[derive(Default)]
pub enum LinkState {
    #[default]
    Disconnected,
    Connected(Box<ActiveLink>),
}

impl LinkState {
    pub fn is_connected(&self) -> bool {
        !matches!(self, Self::Disconnected)
    }

    pub fn kind_label(&self) -> &'static str {
        match self {
            Self::Disconnected => "Offline",
            Self::Connected(link) => link.kind.label(),
        }
    }

    pub fn port_label(&self) -> String {
        match self {
            Self::Disconnected => "—".into(),
            Self::Connected(link) => link.kind.port_label().into(),
        }
    }
}

/// Which transport a connection came up on. Kept as plain data next to
/// the [`Transport`] trait object so UI labels never need downcasting.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum LinkKind {
    Usb,
    Ble,
}

impl LinkKind {
    fn label(self) -> &'static str {
        match self {
            Self::Usb => "USB",
            Self::Ble => "BLE",
        }
    }

    fn port_label(self) -> &'static str {
        match self {
            Self::Usb => "USB serial",
            Self::Ble => "Inkwash (BLE)",
        }
    }
}

/// One live connection: the [`Transport`] trait object plus which radio
/// it runs on.
///
/// The transport lives behind an `Arc` rather than directly inside the
/// box because of invariant 1 above: a caller sending a command needs a
/// short borrow under the `link` mutex, but a caller *waiting* for a
/// reply must not hold that mutex for up to 45s. Cloning the `Arc` under
/// the lock gives every waiter its own handle to keep polling after the
/// lock is released.
pub struct ActiveLink {
    kind: LinkKind,
    transport: Arc<dyn Transport>,
}

impl ActiveLink {
    pub fn new(kind: LinkKind, transport: Arc<dyn Transport>) -> Self {
        Self { kind, transport }
    }

    pub fn kind(&self) -> LinkKind {
        self.kind
    }

    /// Snapshot of the live transport; see the type-level comment about
    /// why this returns an owned clone instead of a borrow.
    pub fn transport(&self) -> Arc<dyn Transport> {
        Arc::clone(&self.transport)
    }
}

/// Result slot shared between the request that actually drove the device
/// and any duplicate callers coalesced onto it.
pub(crate) struct InflightWaiter {
    result: Mutex<Option<Result<DeviceCommandResult, AppError>>>,
    signal: Condvar,
}

impl InflightWaiter {
    /// Blocks until the leader publishes its result or `deadline`
    /// passes.
    pub(crate) fn wait(&self, deadline: Instant) -> Result<DeviceCommandResult, AppError> {
        let mut slot = self.result.lock().expect("inflight result mutex poisoned");
        loop {
            if let Some(result) = &*slot {
                return result.clone();
            }
            let now = Instant::now();
            if now >= deadline {
                return Err(AppError::device_timeout());
            }
            let (guard, _) = self
                .signal
                .wait_timeout(slot, deadline - now)
                .expect("inflight result mutex poisoned");
            slot = guard;
        }
    }
}

/// Registry of the one in-flight device request. Identical concurrent
/// commands are coalesced: the first talks to the device, later identical
/// ones wait for and share its result instead of sending their own copy.
pub struct InflightRegistry {
    current: Mutex<Option<(String, Arc<InflightWaiter>)>>,
}

impl InflightRegistry {
    pub fn new() -> Self {
        Self {
            current: Mutex::new(None),
        }
    }

    /// Registers `key` as in flight. If an identical key is already
    /// registered, returns [`InflightRegistration::Joined`] with a handle
    /// to that request's shared result; otherwise returns
    /// [`InflightRegistration::Leader`] whose guard publishes the result
    /// and deregisters on drop.
    pub(crate) fn register(&self, key: &str) -> InflightRegistration<'_> {
        let mut current = self.current.lock().expect("inflight registry poisoned");
        if let Some((existing, waiter)) = &*current {
            if existing == key {
                return InflightRegistration::Joined(Arc::clone(waiter));
            }
        }
        let waiter = Arc::new(InflightWaiter {
            result: Mutex::new(None),
            signal: Condvar::new(),
        });
        *current = Some((key.to_string(), Arc::clone(&waiter)));
        InflightRegistration::Leader(InflightGuard {
            registry: self,
            waiter,
            key: key.to_string(),
        })
    }
}

pub(crate) enum InflightRegistration<'a> {
    Leader(InflightGuard<'a>),
    Joined(Arc<InflightWaiter>),
}

/// RAII guard for the request that actually drives the device. On drop it
/// wakes any joiners and deregisters - even on early-return error paths
/// where [`InflightGuard::finish`] was never called.
pub(crate) struct InflightGuard<'a> {
    registry: &'a InflightRegistry,
    waiter: Arc<InflightWaiter>,
    key: String,
}

impl InflightGuard<'_> {
    pub(crate) fn finish(self, result: Result<DeviceCommandResult, AppError>) {
        let mut slot = self
            .waiter
            .result
            .lock()
            .expect("inflight result mutex poisoned");
        *slot = Some(result);
        drop(slot);
        self.waiter.signal.notify_all();
    }
}

impl Drop for InflightGuard<'_> {
    fn drop(&mut self) {
        // Never leave joiners hanging if finish() wasn't reached.
        let mut slot = self
            .waiter
            .result
            .lock()
            .expect("inflight result mutex poisoned");
        if slot.is_none() {
            *slot = Some(Err(AppError::internal("device request was abandoned")));
        }
        drop(slot);
        self.waiter.signal.notify_all();
        // Deregister unless a newer leader already took the slot.
        let mut current = self.registry.current.lock().expect("inflight registry poisoned");
        if matches!(&*current, Some((key, waiter)) if *key == self.key && Arc::ptr_eq(waiter, &self.waiter))
        {
            *current = None;
        }
    }
}

#[cfg(test)]
mod inflight_tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn identical_key_joins_and_shares_leaders_result() {
        let registry = InflightRegistry::new();
        let leader = match registry.register("probe") {
            InflightRegistration::Leader(guard) => guard,
            InflightRegistration::Joined(_) => panic!("first registration must lead"),
        };
        let waiter = match registry.register("probe") {
            InflightRegistration::Joined(waiter) => waiter,
            InflightRegistration::Leader(_) => panic!("identical key must join"),
        };

        let handle =
            std::thread::spawn(move || waiter.wait(Instant::now() + Duration::from_secs(1)));
        std::thread::sleep(Duration::from_millis(50));
        leader.finish(Ok(DeviceCommandResult {
            kind: "ok".into(),
            message: "shared".into(),
            status: None,
        }));
        let joined = handle.join().unwrap().expect("leader's result");
        assert_eq!(joined.kind, "ok");
        assert_eq!(joined.message, "shared");
    }

    #[test]
    fn joiner_times_out_when_leader_never_finishes() {
        let registry = InflightRegistry::new();
        let _leader = match registry.register("probe") {
            InflightRegistration::Leader(guard) => guard,
            InflightRegistration::Joined(_) => panic!("first registration must lead"),
        };
        let waiter = match registry.register("probe") {
            InflightRegistration::Joined(waiter) => waiter,
            InflightRegistration::Leader(_) => panic!("identical key must join"),
        };
        // Deadline already in the past: the wait must fail fast, not hang.
        let result = waiter.wait(Instant::now() - Duration::from_millis(1));
        assert_eq!(result.unwrap_err().code, "DEVICE_TIMEOUT");
    }

    #[test]
    fn dropped_leader_frees_the_slot_without_finishing() {
        let registry = InflightRegistry::new();
        {
            let leader = match registry.register("probe") {
                InflightRegistration::Leader(guard) => guard,
                InflightRegistration::Joined(_) => panic!("first registration must lead"),
            };
            let waiter = match registry.register("probe") {
                InflightRegistration::Joined(waiter) => waiter,
                InflightRegistration::Leader(_) => panic!("identical key must join"),
            };
            // Joiner must not hang past its deadline when the leader is
            // simply dropped without finishing.
            std::thread::spawn(move || {
                waiter.wait(Instant::now() + Duration::from_secs(1))
            });
            drop(leader);
        }
        assert!(matches!(
            registry.register("probe"),
            InflightRegistration::Leader(_)
        ));
    }
}

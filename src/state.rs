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

use std::sync::Mutex;
use tauri::AppHandle;

use crate::commands::logs::LogStore;
use crate::transport::Transport;

pub struct AppState {
    pub link: Mutex<LinkState>,
    pub logs: LogStore,
    pub ctx: AppHandle,
}

impl AppState {
    pub fn new(ctx: AppHandle) -> Self {
        let logs = LogStore::new(ctx.clone());
        logs.info("app", "Inkwash Desktop started");
        Self {
            link: Mutex::new(LinkState::Disconnected),
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
    transport: std::sync::Arc<dyn Transport>,
}

impl ActiveLink {
    pub fn new(kind: LinkKind, transport: std::sync::Arc<dyn Transport>) -> Self {
        Self { kind, transport }
    }

    pub fn kind(&self) -> LinkKind {
        self.kind
    }

    /// Snapshot of the live transport; see the type-level comment about
    /// why this returns an owned clone instead of a borrow.
    pub fn transport(&self) -> std::sync::Arc<dyn Transport> {
        std::sync::Arc::clone(&self.transport)
    }
}


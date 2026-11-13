//! Keeping this computer awake while another machine uses it
//! (docs/PLAN-MACHINES.md §5.14): a power assertion the app holds — no
//! system setting changes. macOS: a `caffeinate -i -w <our pid>` child (it
//! ends with the app); Windows: a thread holding `SetThreadExecutionState`.

use std::sync::atomic::Ordering;
use std::sync::Mutex;

use crate::state::AppState;

/// The assertion, when held.
#[derive(Default)]
pub struct KeepAwake {
    hold: Mutex<Option<Hold>>,
}

enum Hold {
    #[cfg(target_os = "macos")]
    Caffeinate(std::process::Child),
    #[cfg(windows)]
    Thread(std::sync::mpsc::Sender<()>),
    #[allow(dead_code)]
    Nothing,
}

impl KeepAwake {
    pub fn held(&self) -> bool {
        self.hold
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .is_some()
    }

    fn set(&self, on: bool) {
        let mut hold = self.hold.lock().unwrap_or_else(|e| e.into_inner());
        match (on, hold.is_some()) {
            (true, false) => *hold = acquire(),
            (false, true) => {
                if let Some(h) = hold.take() {
                    release(h);
                }
            }
            _ => {}
        }
    }
}

impl Drop for KeepAwake {
    fn drop(&mut self) {
        if let Some(h) = self.hold.get_mut().ok().and_then(Option::take) {
            release(h);
        }
    }
}

/// Holds the assertion while the setting is on and a paired machine is
/// connected (only in the app: tests never touch the power settings).
pub fn update(state: &AppState) {
    let live = state.machines.fixes_live.load(Ordering::Relaxed);
    let want = live && state.machines.settings().keep_awake && state.machines.any_connected();
    state.machines.awake.set(want);
}

fn acquire() -> Option<Hold> {
    #[cfg(target_os = "macos")]
    {
        let pid = std::process::id().to_string();
        match crate::proc::std_command("/usr/bin/caffeinate")
            .args(["-i", "-w", &pid])
            .spawn()
        {
            Ok(child) => Some(Hold::Caffeinate(child)),
            Err(e) => {
                tracing::warn!(error = %e, "could not keep this Mac awake");
                None
            }
        }
    }
    #[cfg(windows)]
    {
        use windows_sys::Win32::System::Power::{
            SetThreadExecutionState, ES_CONTINUOUS, ES_SYSTEM_REQUIRED,
        };
        let (tx, rx) = std::sync::mpsc::channel::<()>();
        let spawned = std::thread::Builder::new()
            .name("machines-keep-awake".into())
            .spawn(move || {
                // SAFETY: plain flags; the state belongs to this thread.
                unsafe { SetThreadExecutionState(ES_CONTINUOUS | ES_SYSTEM_REQUIRED) };
                let _ = rx.recv();
                // SAFETY: as above.
                unsafe { SetThreadExecutionState(ES_CONTINUOUS) };
            });
        match spawned {
            Ok(_) => Some(Hold::Thread(tx)),
            Err(e) => {
                tracing::warn!(error = %e, "could not keep this PC awake");
                None
            }
        }
    }
    #[cfg(not(any(target_os = "macos", windows)))]
    {
        Some(Hold::Nothing)
    }
}

fn release(hold: Hold) {
    match hold {
        #[cfg(target_os = "macos")]
        Hold::Caffeinate(mut child) => {
            let _ = child.kill();
            let _ = child.wait();
        }
        #[cfg(windows)]
        Hold::Thread(tx) => {
            let _ = tx.send(());
        }
        Hold::Nothing => {}
    }
}

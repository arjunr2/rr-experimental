//! Crimp glue trace reader.
//!
//! Provides exported FFI functions for opening recorded trace files
//! and iterating over deserialized events. Intended to be compiled to
//! Wasm and linked from another language.

mod convert;
mod types;

use std::ffi::{CStr, c_char};
use std::fs::File;
use std::mem::ManuallyDrop;

pub use wasm_crimp::{RREvent, RecordSettings, ReplaySettings, from_replay_reader};

pub use types::*;

// ── Internal replayer ───────────────────────────────────────────────

/// Opaque replayer state. FFI consumers interact with this only through
/// raw pointers returned by [`crimp_replayer_open`].
pub struct CrimpReplayer {
    reader: File,
    scratch: Vec<u8>,
    #[allow(dead_code)]
    settings: ReplaySettings,
    #[allow(dead_code)]
    trace_settings: RecordSettings,
    eof: bool,
}

impl CrimpReplayer {
    /// Open a trace file and read the initial `TraceSignatureEvent`.
    fn open(path: &str, settings: ReplaySettings) -> anyhow::Result<Self> {
        let file = File::open(path)?;
        let mut replayer = CrimpReplayer {
            reader: file,
            scratch: vec![0u8; settings.deserialize_buffer_size],
            settings,
            trace_settings: RecordSettings::default(),
            eof: false,
        };

        // Read and validate the trace signature (always the first event)
        let sig_event = from_replay_reader(&mut replayer.reader, &mut replayer.scratch)?;
        if let RREvent::TraceSignature(sig) = sig_event {
            replayer.trace_settings = sig.settings;
        } else {
            anyhow::bail!("expected TraceSignatureEvent as first event, got: {sig_event}");
        }

        Ok(replayer)
    }

    /// Read the next event, skipping diagnostic events (Nop, CustomMessage).
    fn next_event(&mut self) -> CrimpEvent {
        if self.eof {
            return CrimpEvent {
                tag: CrimpEventTag::Eof,
                payload: CrimpEventPayload { none: () },
            };
        }

        loop {
            match from_replay_reader(&mut self.reader, &mut self.scratch) {
                Ok(event) => {
                    if event.is_diagnostic() {
                        continue;
                    }
                    if matches!(event, RREvent::Eof) {
                        self.eof = true;
                    }
                    return CrimpEvent::from(event);
                }
                Err(e) => {
                    self.eof = true;
                    return CrimpEvent {
                        tag: CrimpEventTag::Error,
                        payload: CrimpEventPayload {
                            error_msg: ManuallyDrop::new(CrimpString::from(e.to_string())),
                        },
                    };
                }
            }
        }
    }
}

// ── FFI functions ───────────────────────────────────────────────────

/// Open a trace file and create a replayer.
///
/// # Parameters
/// - `filepath`: Null-terminated C string path to the trace file.
/// - `settings`: Replay settings.
///
/// # Returns
/// An opaque pointer to the replayer, or null on error.
///
/// # Safety
/// `filepath` must be a valid, null-terminated C string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn crimp_replayer_open(
    filepath: *const c_char,
    settings: CrimpReplaySettings,
) -> *mut CrimpReplayer {
    if filepath.is_null() {
        return std::ptr::null_mut();
    }

    let path = match unsafe { CStr::from_ptr(filepath) }.to_str() {
        Ok(s) => s,
        Err(_) => return std::ptr::null_mut(),
    };

    let replay_settings = ReplaySettings {
        validate: settings.validate,
        deserialize_buffer_size: settings.deserialize_buffer_size,
    };

    match CrimpReplayer::open(path, replay_settings) {
        Ok(replayer) => Box::into_raw(Box::new(replayer)),
        Err(_) => std::ptr::null_mut(),
    }
}

/// Read the next event from the replayer.
///
/// Returns an event with `tag == CrimpEventTag::Eof` at the end of the
/// trace, or `tag == CrimpEventTag::Error` on a read failure.
///
/// The caller **must** call [`crimp_event_free`] on the returned event
/// to release any heap-allocated payload data.
///
/// # Safety
/// `replayer` must be a valid pointer returned by [`crimp_replayer_open`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn crimp_replayer_next_event(replayer: *mut CrimpReplayer) -> CrimpEvent {
    if replayer.is_null() {
        return CrimpEvent {
            tag: CrimpEventTag::Error,
            payload: CrimpEventPayload {
                error_msg: ManuallyDrop::new(CrimpString::from(
                    "null replayer pointer".to_string(),
                )),
            },
        };
    }

    let replayer = unsafe { &mut *replayer };
    replayer.next_event()
}

/// Free heap-allocated data within a `CrimpEvent`.
///
/// This does **not** free the `CrimpEvent` struct itself (it is returned
/// by value). It only frees owned heap data inside the payload
/// (buffers, strings, etc.).
///
/// # Safety
/// `event` must point to a valid `CrimpEvent`. Must only be called once
/// per event.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn crimp_event_free(event: *mut CrimpEvent) {
    if !event.is_null() {
        unsafe {
            (*event).free();
        }
    }
}

/// Free a replayer instance.
///
/// # Safety
/// `replayer` must be a valid pointer returned by [`crimp_replayer_open`],
/// or null (in which case this is a no-op).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn crimp_replayer_free(replayer: *mut CrimpReplayer) {
    if !replayer.is_null() {
        unsafe {
            drop(Box::from_raw(replayer));
        }
    }
}

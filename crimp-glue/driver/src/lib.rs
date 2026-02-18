//! CRIMP replay driver for decomposed WebAssembly modules.
//!
//! This crate provides a single entry point [`run_replay`] that provides the
//! core structure for a replay driver. This is intended to be compiled to Wasm and
//! linked with decomposed replay programs to minimize the work needed to support replay
//! in different engines. The logic reads from a trace file, deserializes and
//! dispatches events accordingly.
//!
//! Note that this is not intended to be a complete implementation, but rather a scaffolding
//! program that is further specialized to a set of components (and optionally, even a trace file
//! if the entire trace is available ahead of time).

use anyhow::Result;
use core::panic;
use env_logger;
use std::fs::File;
use std::io::BufReader;
use std::mem::MaybeUninit;
use wasm_crimp::{
    EventError, ExportIndex, RRComponentInstanceId, RRModuleInstanceId, common_events,
};
use wasm_crimp::{
    RREvent, RecordSettings, ReplayError, ReplayReader, ReplaySettings, Replayer,
    from_replay_reader,
};

#[cfg(feature = "multi-component")]
compile_error!("Multi-component support is not yet implemented in the Wasm replay driver.");

const TRACE_FILEPATH: &str = env!("TRACE_FILEPATH");
const DESERIALIZE_BUFFER_SIZE: Option<&str> = option_env!("DESERIALIZE_BUFFER_SIZE"); // 1 MiB buffer for deserialization

// ===================================================================================
// Global State
// ===================================================================================
static mut REPLAYER: MaybeUninit<ReplayBuffer> = MaybeUninit::uninit();

macro_rules! access (
    ($replayer:ident) => (
        unsafe {
            let raw_ptr = std::ptr::addr_of_mut!($replayer);
            (*raw_ptr).assume_init_mut()
        }
    )
);

// ===================================================================================
// Import helpers from the glue code
// ===================================================================================
#[cfg(feature = "glue")]
#[link(wasm_import_module = "crimp_glue")]
unsafe extern "C" {
    /// Get the checksum of the currently instantiated component/module being driven.
    ///
    /// The caller is expected to populate the buffer with 32 bytes to write the checksum into.
    /// This is to avoid dynamic memory allocation in the driver.
    fn get_sha256_checksum(checksum_buf: *mut u8);

    /// A dispatch method that calls the appropriate realloc for the given export's lowering.
    fn dispatch_realloc(
        export_index: u32,
        old_addr: u32,
        old_size: u32,
        old_align: u32,
        new_size: u32,
    ) -> u32;

    /// A dispatch method that calls the appropriate memory write for the given export's lowering.
    fn dispatch_memory_write(export_index: u32, offset: u32, bytes_ptr: *const u8, num_bytes: u32);

    /// A dispatch method that calls the appropriate core function post-lowering function for the given export.
    ///
    /// The glue logic already has the signature so we don't need to pass them, just the pointer to the encoded
    /// recorded return values is sufficient. `num_args` is only passed now for a simple assertion.
    fn dispatch_core_func(export_index: u32, args: *const u8, num_args: u32) -> u64;
}
/// ===================================================================================
/// [`ReplayBuffer`] implementing a [`Replayer`] (copied mostly implementation from Wasmtime)
/// ===================================================================================

/// Buffer to read replay data
pub struct ReplayBuffer {
    /// Reader to read replay trace from
    reader: Box<dyn ReplayReader>,
    /// Settings in replay configuration
    settings: ReplaySettings,
    /// Settings for record configuration (encoded in the trace)
    trace_settings: RecordSettings,
    /// Intermediate static buffer for deserialization
    deser_buffer: Vec<u8>,
    /// Whether buffer has been completely read
    eof_encountered: bool,
}

impl Iterator for ReplayBuffer {
    type Item = Result<RREvent, ReplayError>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.eof_encountered {
            return None;
        }
        let ret = 'event_loop: loop {
            let result = from_replay_reader(&mut *self.reader, &mut self.deser_buffer);
            match result {
                Err(e) => {
                    break 'event_loop Some(Err(ReplayError::FailedRead(e)));
                }
                Ok(event) => {
                    if let RREvent::Eof = &event {
                        self.eof_encountered = true;
                        break 'event_loop None;
                    } else if event.is_diagnostic() {
                        continue 'event_loop;
                    } else {
                        log::debug!("Read replay event => {event}");
                        break 'event_loop Some(Ok(event));
                    }
                }
            }
        };
        ret
    }
}

impl Replayer for ReplayBuffer {
    fn new_replayer(reader: impl ReplayReader + 'static, settings: ReplaySettings) -> Result<Self> {
        let mut buf = ReplayBuffer {
            reader: Box::new(reader),
            deser_buffer: vec![0; settings.deserialize_buffer_size],
            settings,
            // This doesn't matter now; will override after reading header
            trace_settings: RecordSettings::default(),
            eof_encountered: false,
        };

        let signature: common_events::TraceSignatureEvent = buf.next_event_typed()?;
        // NOTE: Trace checksum is not needed to be validated here since this replay
        // format is supposed to be indepedent of the Engine.

        // Update the trace settings
        buf.trace_settings = signature.settings;

        if buf.settings.validate && !buf.trace_settings.add_validation {
            log::warn!(
                "Replay validation will be omitted since the recorded trace has no validation metadata..."
            );
        }

        Ok(buf)
    }

    #[inline]
    fn settings(&self) -> &ReplaySettings {
        &self.settings
    }

    #[inline]
    fn trace_settings(&self) -> &RecordSettings {
        &self.trace_settings
    }
}

/// ===================================================================================
/// Helper and glue export methods for main replayer
/// ===================================================================================
enum Instance {
    Component(RRComponentInstanceId),
    Module(RRModuleInstanceId),
}

fn check_instance(
    instance: &Option<Instance>,
    read_checksum: [u8; 32],
    expected_checksum: [u8; 32],
) {
    if instance.is_some() {
        panic!(
            "Multiple instantiations not supported in this feature set. Consider the `multi-component` feature"
        );
    }
    #[cfg(feature = "glue")]
    assert_eq!(
        read_checksum, expected_checksum,
        "Checksum in trace and component do not match. Ensure CHECKSUM env variable is set correctly."
    );
}

fn throw_event_error(error: impl EventError) -> ! {
    panic!("Replay encountered a EventError in Trace: {}", error);
}

/// Lowering logic for Wasm function calls
unsafe fn replay_wasm_call(export_index: ExportIndex) {
    let mut realloc_return_stack: Vec<u32> = Vec::new();
    while let Some(event_res) = access!(REPLAYER).next() {
        let event = event_res.unwrap();
        match event {
            RREvent::ComponentLowerFlatEntry(_) | RREvent::ComponentLowerMemoryEntry(_) => {
                log::warn!(
                    "Lowering entry validation cannot currently be performed in the Wasm replay driver, ignoring....."
                );
            }
            RREvent::ComponentReallocEntry(e) => {
                #[cfg(feature = "glue")]
                unsafe {
                    realloc_return_stack.push(dispatch_realloc(
                        export_index.as_u32(),
                        e.old_addr.try_into().unwrap(),
                        e.old_size.try_into().unwrap(),
                        e.old_align,
                        e.new_size.try_into().unwrap(),
                    ));
                }
            }
            RREvent::ComponentReallocReturn(e) => match e.0.ret() {
                Ok(r) => {
                    #[cfg(feature = "glue")]
                    {
                        let r32: u32 = r.try_into().unwrap();
                        assert_eq!(
                            r32,
                            realloc_return_stack.pop().unwrap(),
                            "Realloc return value does not match the recorded return value!"
                        );
                    }
                }
                Err(x) => throw_event_error(x),
            },
            RREvent::ComponentMemorySliceWrite(e) => {}
            RREvent::ComponentLowerFlatReturn(e) => {
                if let Err(x) = e.0.ret() {
                    throw_event_error(x);
                }
            }
            RREvent::ComponentWasmFuncEntry(e) => {
                #[cfg(feature = "glue")]
                unsafe {
                    dispatch_core_func(
                        export_index.as_u32(),
                        e.args.bytes.as_ptr(),
                        e.args.sizes.len() as u32,
                    );
                }
                break;
            }
            RREvent::WasmFuncReturn(e) => match e.0.ret() {
                Ok(r) => {
                    log::info!(
                        "Wasm function returned successfully with return value: {:?}",
                        r
                    );
                }
                Err(x) => throw_event_error(x),
            },
            _ => {
                panic!("Invalid event {:?} encountered in wasm call replay!", event);
            }
        }
    }
    assert!(realloc_return_stack.is_empty());
}

// ===================================================================================
// Exported methods for glue modules to call
// ===================================================================================
/// Trace following for lowering logic for import calls from Wasm to Host
///
/// The glue logic already has the signature so we don't need to pass them, just the pointer to the encoded
/// recorded return values is sufficient. `num_args` is only passed now for a simple assertion.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn replay_host_call() -> *mut u8 {
    //while let Some(event_res) = access!(REPLAYER).next() {
    //    let event = event_res.unwrap();
    //}
    panic!("Host call replay is not yet implemented in the Wasm replay driver!");
    std::ptr::null_mut()
}

/// Trace following for builtin calls from Wasm, similar to [`replay_host_call`]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn replay_builtin_call() -> *mut u8 {
    //while let Some(event_res) = access!(REPLAYER).next() {
    //    let event = event_res.unwrap();
    //}
    panic!("Builtin call is not yet implemented in the Wasm replay driver!");
    std::ptr::null_mut()
}

/// The main entrypoint for the replay driver, intended to be called from the Wasm engine
#[unsafe(no_mangle)]
pub unsafe extern "C" fn run_replay() {
    env_logger::init();
    log::debug!("Trace file: {}", TRACE_FILEPATH);
    let file = File::open(TRACE_FILEPATH)
        .expect(&format!("Failed to open trace file: {}", TRACE_FILEPATH));

    let replayer = std::ptr::addr_of_mut!(REPLAYER);
    // Initialize the global replayer state
    unsafe {
        (*replayer).write(
            ReplayBuffer::new_replayer(
                BufReader::new(file),
                ReplaySettings {
                    // For now, we don't support validation in wasm driver
                    validate: false,
                    deserialize_buffer_size: DESERIALIZE_BUFFER_SIZE
                        .and_then(|s| s.parse::<usize>().ok())
                        .unwrap_or(1024), // Default to 1 KiB if not set or invalid
                },
            )
            .unwrap(),
        );
    }

    let mut instance: Option<Instance> = None;
    let mut expected_checksum: [u8; 32] = Default::default();
    #[cfg(feature = "glue")]
    unsafe {
        get_sha256_checksum(expected_checksum.as_mut_ptr());
    }

    // Top-level events: Wasm to Host function calls
    while let Some(event_res) = access!(REPLAYER).next() {
        let event = event_res.unwrap();
        match event {
            // Instantiation events
            RREvent::ComponentInstantiation(e) => {
                check_instance(&instance, *e.component, expected_checksum);
                instance = Some(Instance::Component(e.instance));
            }
            RREvent::CoreWasmInstantiation(e) => {
                check_instance(&instance, *e.module, expected_checksum);
                instance = Some(Instance::Module(e.instance));
            }
            // Host to Wasm function call events
            RREvent::ComponentWasmFuncBegin(e) => unsafe {
                replay_wasm_call(e.func_index);
            },
            RREvent::ComponentPostReturn(e) => unsafe {
                replay_wasm_call(e.func_index);
            },
            RREvent::CoreWasmFuncEntry(e) => {
                panic!("Core wasm function calls not supported yet..");
            }
            _ => {
                #[cfg(feature = "glue")]
                panic!(
                    "Invalid event {:?} encountered in top-level replay driver!",
                    event
                );
            }
        }
    }
    // Cleanup the replayer state
    unsafe {
        (*replayer).assume_init_drop();
    }
    log::info!("Replay completed successfully!");
}

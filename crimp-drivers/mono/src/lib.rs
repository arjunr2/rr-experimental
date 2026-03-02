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
//!
//! Some todos that could improve performance:
//! * Whenever host_call_return or builtin_return wants to send back args, it overwrites the
//!   backing buffer completely. This includes any expansions to its capacity, so this technique
//!   may suffer from frequent reallocations.

use core::panic;
use env_logger;
use std::collections::HashMap;
use std::fs::File;
use std::io::BufReader;
use std::mem::MaybeUninit;
use wasm_crimp::component_events::{self, LowerFlatEntryEvent, LowerMemoryEntryEvent};
use wasm_crimp::{
    EventError, ExportIndex, RRComponentInstanceId, RRFuncArgVals, RRModuleInstanceId, Validate,
    common_events, core_events,
};
use wasm_crimp::{RREvent, ReplaySettings, Replayer};

mod replayer;
use replayer::*;

mod imports;
use imports::*;

#[cfg(feature = "multi-component")]
compile_error!("Multi-component support is not yet implemented in the Wasm replay driver.");

const TRACE_FILEPATH: Option<&str> = option_env!("TRACE_FILEPATH");
const DESERIALIZE_BUFFER_SIZE: Option<&str> = option_env!("DESERIALIZE_BUFFER_SIZE"); // 1 MiB buffer for deserialization

// ===================================================================================
// Global State
// ===================================================================================

/// The state of the replay driver to determine the current context
#[derive(Debug, Copy, Clone)]
#[allow(dead_code)]
enum State {
    /// Inside the host, but not currently executing any calls from Wasm
    Root,
    /// Setup of lowerings before invoking the target wasm function
    WasmCallLoweringSetup {
        /// Unified export index of target wasm function
        export_index: ExportIndex,
    },
    /// Replay of host call lowering and return values
    HostCall {
        /// Import index
        import_index: u32,
    },
    /// Replay of builtin call return values
    BuiltinCall {
        /// Import index
        import_index: u32,
    },
}

static mut REPLAYER: MaybeUninit<ReplayBuffer> = MaybeUninit::uninit();
/// State tracking for driving the replay state machine.
///
/// NOTE: If supporting multi-component, you need one of these per component
static mut STATE: MaybeUninit<State> = MaybeUninit::uninit();
/// Buffer to move data between glue and driver
///
/// The data in here is shared and set by both the driver and glue logic through a number of safety assumptions.
/// In general, the glue logic uses the [`ARGS_RESULTS_FFI`] to set this and is read back by the driver
/// for validation and post-return calls. The glue logic is responsible for ensuring that the data in this buffer
/// is valid and correctly encoded.
static mut ARGS_RESULTS_BACKING: RRFuncArgVals = RRFuncArgVals {
    bytes: vec![],
    sizes: vec![],
};

macro_rules! access (
    ($global:ident) => (
        unsafe {
            let raw_ptr = std::ptr::addr_of_mut!($global);
            (*raw_ptr).assume_init_mut()
        }
    )
);

/// ===================================================================================
/// Helper and glue export methods for main replayer
/// ===================================================================================
#[allow(dead_code)]
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
    assert_eq!(
        read_checksum, expected_checksum,
        "Checksum in trace and component do not match. Ensure CHECKSUM env variable is set correctly."
    );
}

fn throw_event_error(error: impl EventError) -> ! {
    panic!("Replay encountered a EventError in Trace: {}", error);
}
use anyhow;
fn throw_anyhow_error(error: anyhow::Error) -> ! {
    panic!("Replay encountered a AnyhowError in Trace: {}", error);
}

/// ===================================================================================
/// Event handlers
/// ===================================================================================
#[inline(always)]
#[allow(unused)]
fn lower_flat_entry(event: LowerFlatEntryEvent, state: State) {
    log::debug!(
        "Lowering entry validation (flat) cannot currently be performed in the Wasm replay driver, ignoring...."
    );
}

#[inline(always)]
#[allow(unused)]
fn lower_memory_entry(event: LowerMemoryEntryEvent, state: State) {
    log::debug!(
        "Lowering entry validation (memory) cannot currently be performed in the Wasm replay driver, ignoring...."
    );
}

/// Returns the value for post_return calls
#[inline(always)]
fn component_wasm_func_entry(
    event: component_events::WasmFuncEntryEvent,
    state: State,
) -> (ExportIndex, RRFuncArgVals) {
    match state {
        State::WasmCallLoweringSetup { export_index } => unsafe {
            let (mut param_bytes_len, mut params_sizes_len) = (0u32, 0u32);
            dispatch_core_func(
                export_index.as_u32(),
                event.args.bytes.as_ptr(),
                &raw mut param_bytes_len,
                &raw mut params_sizes_len,
            );
            // Set the backing buffer based on lengths that glue logic encoded with
            let backing = &mut *&raw mut ARGS_RESULTS_BACKING;
            backing.bytes.set_len(param_bytes_len as usize);
            backing.sizes.set_len(params_sizes_len as usize);
            // Need to clone here since the post_return could be executed later in the trace
            (export_index, backing.clone())
        },
        _ => panic!("Invalid state: {:?}", state),
    }
}

/// Push the realloc return value to the stack for the future realloc return event to pop and validate
#[inline(always)]
fn realloc_entry(event: component_events::ReallocEntryEvent, state: State, rstack: &mut Vec<u32>) {
    let (direction, index) = match state {
        State::WasmCallLoweringSetup { export_index } => {
            (LoweringDirection::Export, export_index.as_u32())
        }
        State::HostCall { import_index } => (LoweringDirection::Import, import_index),
        _ => panic!("Invalid state: {:?}", state),
    };
    rstack.push(unsafe {
        dispatch_realloc(
            direction,
            index,
            event.old_addr as u32,
            event.old_size as u32,
            event.old_align,
            event.new_size as u32,
        )
    });
}

/// Validate the realloc return value against the last realloc (top of the stack)
#[inline(always)]
fn realloc_return(
    event: component_events::ReallocReturnEvent,
    state: State,
    rstack: &mut Vec<u32>,
) {
    match state {
        State::WasmCallLoweringSetup { export_index: _ } | State::HostCall { import_index: _ } => {
            let expected = event.0.ret().unwrap_or_else(|e| throw_event_error(e)) as u32;
            expected.validate(&rstack.pop().unwrap()).unwrap();
        }
        _ => panic!("Invalid state: {:?}", state),
    }
}

#[inline(always)]
fn memory_slice_write(event: component_events::MemorySliceWriteEvent, state: State) {
    let (direction, index) = match state {
        State::WasmCallLoweringSetup { export_index } => {
            (LoweringDirection::Export, export_index.as_u32())
        }
        State::HostCall { import_index } => (LoweringDirection::Import, import_index),
        _ => panic!("Invalid state: {:?}", state),
    };
    unsafe {
        dispatch_memory_write(
            direction,
            index,
            event.offset as u32,
            event.bytes.as_ptr(),
            event.bytes.len() as u32,
        );
    }
}

#[inline(always)]
fn lower_flat_return(event: component_events::LowerFlatReturnEvent, state: State) {
    match state {
        State::WasmCallLoweringSetup { export_index: _ } | State::HostCall { import_index: _ } => {
            event.0.ret().unwrap_or_else(|e| throw_event_error(e));
        }
        _ => panic!("Invalid state: {:?}", state),
    }
}

#[inline(always)]
fn lower_memory_return(event: component_events::LowerMemoryReturnEvent, state: State) {
    match state {
        State::WasmCallLoweringSetup { export_index: _ } | State::HostCall { import_index: _ } => {
            event.0.ret().unwrap_or_else(|e| throw_event_error(e));
        }
        _ => panic!("Invalid state: {:?}", state),
    }
}

#[inline(always)]
fn host_func_entry(event: common_events::HostFuncEntryEvent, state: State) {
    match state {
        State::HostCall { import_index: _ } => unsafe {
            event
                .args
                .validate(&*&raw const ARGS_RESULTS_BACKING)
                .unwrap();
        },
        _ => panic!("Invalid state: {:?}", state),
    }
}

#[inline(always)]
fn host_func_return(event: common_events::HostFuncReturnEvent, state: State) -> *mut u8 {
    match state {
        State::HostCall { import_index: _ } => unsafe {
            // Keep the event value alive by moving it into backing.
            let backing = &mut *&raw mut ARGS_RESULTS_BACKING;
            *backing = event.args;
            backing.bytes.as_mut_ptr()
        },
        _ => panic!("Invalid state: {:?}", state),
    }
}

#[inline(always)]
#[allow(unused)]
fn core_wasm_func_entry(event: core_events::WasmFuncEntryEvent, state: State) {
    unreachable!();
}

#[inline(always)]
fn wasm_func_return(event: common_events::WasmFuncReturnEvent, state: State) {
    match state {
        State::Root => unsafe {
            // Set the backing buffer based on lengths that glue logic encoded with
            let backing = &*&raw const ARGS_RESULTS_BACKING;
            let args = event.0.ret().unwrap_or_else(|e| throw_event_error(e));
            args.validate(backing).unwrap();
        },
        _ => panic!("Invalid state: {:?}", state),
    }
}

#[inline(always)]
fn builtin_entry(event: component_events::BuiltinEntryEvent, state: State) {
    use component_events::BuiltinEntryEvent::*;
    match state {
        State::BuiltinCall { .. } => match event {
            ResourceDrop(event) => unsafe {
                let args = RRFuncArgVals {
                    bytes: event.idx.to_le_bytes().to_vec(),
                    sizes: vec![4],
                };
                args.validate(&*&raw const ARGS_RESULTS_BACKING).unwrap();
            },
            _ => {
                panic!("No support for builtin event {:?} yet...", event);
            }
        },
        _ => panic!("Invalid state: {:?}", state),
    }
}

#[inline(always)]
fn builtin_return(event: component_events::BuiltinReturnEvent, state: State) -> *mut u8 {
    use component_events::BuiltinReturnEvent::*;
    match state {
        State::BuiltinCall { import_index: _ } => unsafe {
            let args: RRFuncArgVals;
            match event {
                ResourceDrop(e) => {
                    let ret = e.ret().unwrap_or_else(|e| throw_anyhow_error(e)).0;
                    let mut renc = [0u8; 5];
                    match ret {
                        Some(v) => {
                            renc[..4].copy_from_slice(&v.to_le_bytes());
                            renc[4] = 1u8; // Discriminator for Some
                        } // Encode Some as 1
                        None => {} // Encode None as 0
                    };
                    // Encode the option as 5-bytes (4-byte discrim + 4-byte index)
                    args = RRFuncArgVals {
                        bytes: renc.to_vec(),
                        sizes: vec![5],
                    };
                }
                _ => {
                    panic!("No support for builtin event {:?} yet...", event);
                }
            }
            // Keep the event value alive by moving it into backing.
            let backing = &mut *&raw mut ARGS_RESULTS_BACKING;
            *backing = args;
            backing.bytes.as_mut_ptr()
        },

        _ => panic!("Invalid state: {:?}", state),
    }
}

#[inline(always)]
fn post_return(event: component_events::PostReturnEvent, state: State, args: RRFuncArgVals) {
    match state {
        State::Root => unsafe {
            dispatch_post_return(event.func_index.as_u32(), args.bytes.as_ptr());
        },
        _ => panic!("Invalid state: {:?}", state),
    }
}

#[inline(always)]
fn component_wasm_func_begin(
    event: component_events::WasmFuncBeginEvent,
    state: State,
) -> ExportIndex {
    match state {
        State::Root => event.func_index,
        _ => panic!("Invalid state: {:?}", state),
    }
}

#[inline(always)]
fn component_instantiation(
    event: component_events::InstantiationEvent,
    state: State,
    instance: &mut Option<Instance>,
    expected_checksum: [u8; 32],
) {
    match state {
        State::Root => {
            check_instance(instance, *event.component, expected_checksum);
            *instance = Some(Instance::Component(event.instance));
        }
        _ => panic!("Invalid state: {:?}", state),
    }
}

#[inline(always)]
fn core_wasm_instantiation(
    event: core_events::InstantiationEvent,
    state: State,
    instance: &mut Option<Instance>,
    expected_checksum: [u8; 32],
) {
    match state {
        State::Root => {
            check_instance(instance, *event.module, expected_checksum);
            *instance = Some(Instance::Module(event.instance));
        }
        _ => panic!("Invalid state: {:?}", state),
    }
}

// ===================================================================================
// Exported methods for glue modules to call
// ===================================================================================
/// This points to [`ARGS_RESULTS_BACKING`] when a buffer is allocated and passed to the glue module.
///
/// In general, a pointer to this struct is passed to the glue logic,
/// only because we can't compile multi-value return to Wasm in Rust...!
static mut ARGS_RESULTS_FFI: RRFuncArgValsFFI = RRFuncArgValsFFI {
    bytes_ptr: std::ptr::null_mut(),
    sizes_ptr: std::ptr::null_mut(),
    bytes_len: 0,
    sizes_len: 0,
};
/// This method is intended for the glue logic to write return values from wasm calls and
/// param values for host calls into, so that the replay driver can read and validate them against
/// the trace events.
///
/// This allocates enough space in the backing [`RRFuncArgVals`] for the glue to populate it
/// through the FFI struct. This is then reconstructed back into an [`RRFuncArgVals`] when control
/// is returned to the driver for validation and post-return calls.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn allocate_args_results_buffer(
    total_size: u32,
    num_elements: u32,
) -> *mut RRFuncArgValsFFI {
    let expand = |vec: &mut Vec<u8>, new_size: usize| unsafe {
        vec.clear();
        if vec.capacity() < new_size {
            vec.reserve(new_size - vec.capacity());
        }
        vec.set_len(new_size);
    };
    unsafe {
        let backing = &mut *&raw mut ARGS_RESULTS_BACKING;
        expand(&mut backing.bytes, total_size as usize);
        expand(&mut backing.sizes, num_elements as usize);
        let ffi = &mut *&raw mut ARGS_RESULTS_FFI;
        ffi.bytes_ptr = backing.bytes.as_mut_ptr(); // contents set by glue logic
        ffi.bytes_len = 0; // set by glue logic ONLY on Wasm function returns, ignored for host call params
        ffi.sizes_ptr = backing.sizes.as_mut_ptr(); // contents set by glue logic
        ffi.sizes_len = 0; // set by glue logic ONLY on Wasm function returns, ignored for host call params
        &raw mut ARGS_RESULTS_FFI
    }
}

/// Driver implementation to replay import host call effects from Wasm to Host
///
/// SAFETY: Note here that the glue logic should have already allocated and populated the
/// [`ARGS_RESULTS_BACKING`] buffer with the appropriate parameters when this method is called.
/// It gives back the length so teh driver can make use of that here.
///
/// The import_index passed here should be unified across the entire component.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn replay_host_call(
    import_index: u32,
    params_bytes_len: u32,
    params_sizes_len: u32,
) -> *mut u8 {
    let mut realloc_return_stack: Vec<u32> = Vec::new();
    // Set the backing buffer based on the glue filling in
    unsafe {
        let backing = &mut *&raw mut ARGS_RESULTS_BACKING;
        backing.bytes.set_len(params_bytes_len as usize);
        backing.sizes.set_len(params_sizes_len as usize);
    }

    let state = access!(STATE);
    *state = State::HostCall { import_index };
    while let Some(event_res) = access!(REPLAYER).next() {
        let event = event_res.unwrap();
        match event {
            // Host func boundaries
            RREvent::HostFuncEntry(e) => {
                host_func_entry(e, *state);
            }
            RREvent::HostFuncReturn(e) => {
                // Done
                return host_func_return(e, *state);
            }
            // Lower boundaries
            RREvent::ComponentLowerFlatEntry(e) => {
                lower_flat_entry(e, *state);
            }
            RREvent::ComponentLowerMemoryEntry(e) => {
                lower_memory_entry(e, *state);
            }
            RREvent::ComponentLowerFlatReturn(e) => {
                lower_flat_return(e, *state);
            }
            RREvent::ComponentLowerMemoryReturn(e) => {
                lower_memory_return(e, *state);
            }
            // Lower effects
            RREvent::ComponentReallocEntry(e) => {
                realloc_entry(e, *state, &mut realloc_return_stack);
            }
            RREvent::ComponentReallocReturn(e) => {
                realloc_return(e, *state, &mut realloc_return_stack);
            }
            RREvent::ComponentMemorySliceWrite(e) => {
                memory_slice_write(e, *state);
            }
            // Recursive calls
            RREvent::CoreWasmFuncEntry(e) => {
                core_wasm_func_entry(e, *state);
            }
            RREvent::WasmFuncReturn(e) => {
                wasm_func_return(e, *state);
            }
            _ => {
                panic!("Invalid event {:?} encountered in replay_host_call!", event);
            }
        }
    }
    assert!(realloc_return_stack.is_empty());
    unreachable!("Host function call did not encounter a return event!");
}

/// Trace following for builtin calls from Wasm, similar to [`replay_host_call`]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn replay_builtin_call(
    import_index: u32,
    params_bytes_len: u32,
    params_sizes_len: u32,
) -> *mut u8 {
    // Set the backing buffer based on the glue filling in
    unsafe {
        let backing = &mut *&raw mut ARGS_RESULTS_BACKING;
        backing.bytes.set_len(params_bytes_len as usize);
        backing.sizes.set_len(params_sizes_len as usize);
    }

    let state = access!(STATE);
    *state = State::BuiltinCall { import_index };
    while let Some(event_res) = access!(REPLAYER).next() {
        let event = event_res.unwrap();
        match event {
            // Builtin func boundaries
            RREvent::ComponentBuiltinEntry(e) => {
                builtin_entry(e, *state);
            }
            RREvent::ComponentBuiltinReturn(e) => {
                // Done
                return builtin_return(e, *state);
            }
            _ => {
                panic!(
                    "Invalid event {:?} encountered in replay_builtin_call!",
                    event
                );
            }
        }
    }
    unreachable!("Builtin function call did not encounter a return event!");
}

/// The main entrypoint for the replay driver, intended to be called from the Wasm engine
#[unsafe(no_mangle)]
pub unsafe extern "C" fn run_replay() {
    env_logger::init();
    log::debug!("Trace file: {:?}", TRACE_FILEPATH);
    let filepath = TRACE_FILEPATH.expect("TRACE_FILEPATH environment variable not set. Please set it to the path of the trace file to replay.");
    let file = File::open(filepath).expect("Failed to open trace file");

    let replayer = std::ptr::addr_of_mut!(REPLAYER);
    let state = std::ptr::addr_of_mut!(STATE);

    // Initialize the global replayer state
    unsafe {
        (*replayer).write(
            ReplayBuffer::new_replayer(
                BufReader::new(file),
                ReplaySettings {
                    // The driver just has validate on always
                    // If validation data is not present in trace, a warning is logged
                    validate: true,
                    deserialize_buffer_size: DESERIALIZE_BUFFER_SIZE
                        .and_then(|s| s.parse::<usize>().ok())
                        .unwrap_or(1024), // Default to 1 KiB if not set or invalid
                },
            )
            .unwrap(),
        );
        (*state).write(State::Root);
    }

    let mut instance: Option<Instance> = None;
    let mut expected_checksum: [u8; 32] = Default::default();
    unsafe {
        get_sha256_checksum(expected_checksum.as_mut_ptr());
    }

    let state = access!(STATE);

    let mut realloc_return_stack: Vec<u32> = vec![];
    let mut post_return_args: HashMap<ExportIndex, RRFuncArgVals> = HashMap::new();
    // Top-level events: Wasm to Host function calls
    while let Some(event_res) = access!(REPLAYER).next() {
        let event = event_res.unwrap();
        match event {
            // Instantiation events
            RREvent::ComponentInstantiation(e) => {
                component_instantiation(e, *state, &mut instance, expected_checksum);
            }
            RREvent::CoreWasmInstantiation(e) => {
                core_wasm_instantiation(e, *state, &mut instance, expected_checksum);
            }
            // Host to Wasm function call events (component)
            RREvent::ComponentWasmFuncBegin(e) => {
                let export_index = component_wasm_func_begin(e, *state);
                *state = State::WasmCallLoweringSetup { export_index };
            }
            RREvent::ComponentWasmFuncEntry(e) => {
                let ret = component_wasm_func_entry(e, *state);
                // Save args for future post return call, and we are back in root state
                post_return_args.insert(ret.0, ret.1);
                *state = State::Root;
            }
            RREvent::ComponentPostReturn(e) => {
                let args = post_return_args
                    .remove(&e.func_index)
                    .expect("Post return event for function that was not called!");
                post_return(e, *state, args);
            }
            // Host to Wasm function call events (core)
            RREvent::WasmFuncReturn(e) => {
                wasm_func_return(e, *state);
            }
            RREvent::CoreWasmFuncEntry(e) => {
                core_wasm_func_entry(e, *state);
            }
            // Lower boundaries
            RREvent::ComponentLowerFlatEntry(e) => {
                lower_flat_entry(e, *state);
            }
            RREvent::ComponentLowerFlatReturn(e) => {
                lower_flat_return(e, *state);
            }
            RREvent::ComponentLowerMemoryEntry(e) => {
                lower_memory_entry(e, *state);
            }
            RREvent::ComponentLowerMemoryReturn(e) => {
                lower_memory_return(e, *state);
            }
            // Lower effects
            RREvent::ComponentReallocEntry(e) => {
                realloc_entry(e, *state, &mut realloc_return_stack);
            }
            RREvent::ComponentMemorySliceWrite(e) => {
                memory_slice_write(e, *state);
            }
            RREvent::ComponentReallocReturn(e) => {
                realloc_return(e, *state, &mut realloc_return_stack);
            }
            _ => {
                panic!(
                    "Invalid event {:?} encountered in main loop with state {:?}!",
                    event, state
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

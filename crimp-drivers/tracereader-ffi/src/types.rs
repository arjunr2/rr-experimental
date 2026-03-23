//! C-compatible type definitions for the crimp FFI interface.
//!
//! All types here are `#[repr(C)]` and designed to be consumed by
//! foreign language bindings over the FFI boundary.

use std::ffi::c_char;
use std::mem::ManuallyDrop;

// ── Primitive wrappers ──────────────────────────────────────────────

/// A heap-allocated byte buffer. Owned by Rust; must be freed via
/// [`crimp_event_free`](crate::crimp_event_free).
#[repr(C)]
pub struct CrimpBuffer {
    pub ptr: *mut u8,
    pub len: usize,
}

/// A heap-allocated UTF-8 string (NOT null-terminated).
/// Owned by Rust; must be freed via
/// [`crimp_event_free`](crate::crimp_event_free).
#[repr(C)]
pub struct CrimpString {
    pub ptr: *mut c_char,
    pub len: usize,
}

/// Flat function argument representation (mirrors `RRFuncArgVals`).
#[repr(C)]
pub struct CrimpFuncArgVals {
    pub bytes: CrimpBuffer,
    pub sizes: CrimpBuffer,
}

/// Settings for execution replay (mirrors `ReplaySettings`).
#[repr(C)]
pub struct CrimpReplaySettings {
    pub validate: bool,
    pub deserialize_buffer_size: usize,
}

/// Settings embedded in a recorded trace (mirrors `RecordSettings`).
#[repr(C)]
pub struct CrimpRecordSettings {
    pub add_validation: bool,
}

// ── Result wrappers (one per success type) ──────────────────────────
//
// All EventError types in wasm-crimp are String wrappers, so errors
// are always represented as CrimpString.

/// Result with no success payload.
#[repr(C)]
pub struct CrimpResultVoid {
    pub is_ok: bool,
    pub err_msg: CrimpString,
}

/// Result with `CrimpFuncArgVals` success payload.
#[repr(C)]
pub struct CrimpResultFuncArgVals {
    pub is_ok: bool,
    pub ok_value: CrimpFuncArgVals,
    pub err_msg: CrimpString,
}

/// Result with `usize` success payload.
#[repr(C)]
pub struct CrimpResultUsize {
    pub is_ok: bool,
    pub ok_value: usize,
    pub err_msg: CrimpString,
}

/// Result with `u32` success payload.
#[repr(C)]
pub struct CrimpResultU32 {
    pub is_ok: bool,
    pub ok_value: u32,
    pub err_msg: CrimpString,
}

/// Result with `Option<u32>` success payload (for `ResourceDropRet`).
#[repr(C)]
pub struct CrimpResultOptionU32 {
    pub is_ok: bool,
    pub ok_has_value: bool,
    pub ok_value: u32,
    pub err_msg: CrimpString,
}

// ── InterfaceType (simplified) ──────────────────────────────────────

/// Simplified representation of `wasmtime_environ::component::InterfaceType`.
///
/// Primitive types (Bool=0, S8=1, ...) use `tag` only.
/// Compound types (Record, List, ...) include an `index` payload.
#[repr(C)]
pub struct CrimpInterfaceType {
    pub tag: u32,
    pub index: u32,
}

// ── Event tag enum ──────────────────────────────────────────────────

/// Discriminant for [`CrimpEvent`].
#[repr(C)]
pub enum CrimpEventTag {
    /// End of trace
    Eof = 0,
    /// An error occurred during reading/deserialization
    Error,
    /// No-op marker
    Nop,
    /// Trace signature (always first real event)
    TraceSignature,
    /// Diagnostic custom message
    CustomMessage,
    // Common events
    HostFuncReturn,
    HostFuncEntry,
    WasmFuncReturn,
    // Core module events
    CoreWasmInstantiation,
    CoreWasmFuncEntry,
    // Component events
    ComponentWasmFuncBegin,
    ComponentWasmFuncEntry,
    ComponentInstantiation,
    ComponentReallocEntry,
    ComponentLowerFlatReturn,
    ComponentLowerMemoryReturn,
    ComponentMemorySliceWrite,
    ComponentBuiltinReturn,
    ComponentPostReturn,
    ComponentReallocReturn,
    ComponentLowerFlatEntry,
    ComponentLowerMemoryEntry,
    ComponentBuiltinEntry,
}

// ── Per-event payload structs ───────────────────────────────────────

#[repr(C)]
pub struct CrimpTraceSignatureEvent {
    pub checksum: CrimpString,
    pub settings: CrimpRecordSettings,
}

#[repr(C)]
pub struct CrimpCustomMessageEvent {
    pub message: CrimpString,
}

#[repr(C)]
pub struct CrimpHostFuncReturnEvent {
    pub args: CrimpFuncArgVals,
}

#[repr(C)]
pub struct CrimpHostFuncEntryEvent {
    pub args: CrimpFuncArgVals,
}

#[repr(C)]
pub struct CrimpWasmFuncReturnEvent {
    pub result: CrimpResultFuncArgVals,
}

/// `WasmChecksum` is `[u8; 32]` (SHA-256).
#[repr(C)]
pub struct CrimpCoreInstantiationEvent {
    pub module: [u8; 32],
    pub instance: u32,
}

#[repr(C)]
pub struct CrimpCoreWasmFuncEntryEvent {
    pub instance: u32,
    pub func_index: u32,
    pub args: CrimpFuncArgVals,
}

#[repr(C)]
pub struct CrimpCompWasmFuncBeginEvent {
    pub instance: u32,
    pub func_index: u32,
}

#[repr(C)]
pub struct CrimpCompWasmFuncEntryEvent {
    pub args: CrimpFuncArgVals,
}

#[repr(C)]
pub struct CrimpCompInstantiationEvent {
    pub component: [u8; 32],
    pub instance: u32,
}

#[repr(C)]
pub struct CrimpCompReallocEntryEvent {
    pub old_addr: usize,
    pub old_size: usize,
    pub old_align: u32,
    pub new_size: usize,
}

#[repr(C)]
pub struct CrimpCompLowerFlatReturnEvent {
    pub result: CrimpResultVoid,
}

#[repr(C)]
pub struct CrimpCompLowerMemoryReturnEvent {
    pub result: CrimpResultVoid,
}

#[repr(C)]
pub struct CrimpCompMemorySliceWriteEvent {
    pub offset: usize,
    pub bytes: CrimpBuffer,
}

#[repr(C)]
pub struct CrimpCompPostReturnEvent {
    pub instance: u32,
    pub func_index: u32,
}

#[repr(C)]
pub struct CrimpCompReallocReturnEvent {
    pub result: CrimpResultUsize,
}

#[repr(C)]
pub struct CrimpCompLowerFlatEntryEvent {
    pub ty: CrimpInterfaceType,
}

#[repr(C)]
pub struct CrimpCompLowerMemoryEntryEvent {
    pub ty: CrimpInterfaceType,
    pub offset: usize,
}

// ── Builtin events ──────────────────────────────────────────────────

/// Tag for `BuiltinEntryEvent` variants.
#[repr(C)]
pub enum CrimpBuiltinEntryTag {
    ResourceNew32,
    ResourceRep32,
    ResourceDrop,
    ResourceTransferOwn,
    ResourceTransferBorrow,
    ResourceEnterCall,
    ResourceExitCall,
}

/// C-compatible representation of `BuiltinEntryEvent`.
///
/// Each variant carries up to 3 `u32` parameters from the builtin call
/// (excluding `vmctx`). `param_count` indicates how many are valid.
#[repr(C)]
pub struct CrimpCompBuiltinEntryEvent {
    pub tag: CrimpBuiltinEntryTag,
    pub params: [u32; 3],
    pub param_count: u8,
}

/// Tag for `BuiltinReturnEvent` variants.
#[repr(C)]
pub enum CrimpBuiltinReturnTag {
    ResourceNew32,
    ResourceRep32,
    ResourceDrop,
    ResourceTransferOwn,
    ResourceTransferBorrow,
    ResourceExitCall,
}

/// C-compatible representation of `BuiltinReturnEvent`.
///
/// The success payload depends on the variant:
/// - `ResourceNew32`, `ResourceRep32`, `TransferOwn`, `TransferBorrow` → `ok_u32`
/// - `ResourceDrop` → `ok_has_value` + `ok_u32` (Option<u32>)
/// - `ResourceExitCall` → no success payload (unit)
#[repr(C)]
pub struct CrimpCompBuiltinReturnEvent {
    pub tag: CrimpBuiltinReturnTag,
    pub is_ok: bool,
    pub ok_u32: u32,
    pub ok_has_value: bool,
    pub err_msg: CrimpString,
}

// ── Top-level tagged union ──────────────────────────────────────────

/// Payload union for [`CrimpEvent`].
///
/// Access the field corresponding to `CrimpEvent::tag`. Accessing a
/// different field is undefined behavior.
#[repr(C)]
pub union CrimpEventPayload {
    pub none: (),
    pub error_msg: ManuallyDrop<CrimpString>,
    pub trace_signature: ManuallyDrop<CrimpTraceSignatureEvent>,
    pub custom_message: ManuallyDrop<CrimpCustomMessageEvent>,
    pub host_func_return: ManuallyDrop<CrimpHostFuncReturnEvent>,
    pub host_func_entry: ManuallyDrop<CrimpHostFuncEntryEvent>,
    pub wasm_func_return: ManuallyDrop<CrimpWasmFuncReturnEvent>,
    pub core_instantiation: ManuallyDrop<CrimpCoreInstantiationEvent>,
    pub core_wasm_func_entry: ManuallyDrop<CrimpCoreWasmFuncEntryEvent>,
    pub comp_wasm_func_begin: ManuallyDrop<CrimpCompWasmFuncBeginEvent>,
    pub comp_wasm_func_entry: ManuallyDrop<CrimpCompWasmFuncEntryEvent>,
    pub comp_instantiation: ManuallyDrop<CrimpCompInstantiationEvent>,
    pub comp_realloc_entry: ManuallyDrop<CrimpCompReallocEntryEvent>,
    pub comp_lower_flat_return: ManuallyDrop<CrimpCompLowerFlatReturnEvent>,
    pub comp_lower_memory_return: ManuallyDrop<CrimpCompLowerMemoryReturnEvent>,
    pub comp_memory_slice_write: ManuallyDrop<CrimpCompMemorySliceWriteEvent>,
    pub comp_builtin_return: ManuallyDrop<CrimpCompBuiltinReturnEvent>,
    pub comp_post_return: ManuallyDrop<CrimpCompPostReturnEvent>,
    pub comp_realloc_return: ManuallyDrop<CrimpCompReallocReturnEvent>,
    pub comp_lower_flat_entry: ManuallyDrop<CrimpCompLowerFlatEntryEvent>,
    pub comp_lower_memory_entry: ManuallyDrop<CrimpCompLowerMemoryEntryEvent>,
    pub comp_builtin_entry: ManuallyDrop<CrimpCompBuiltinEntryEvent>,
}

/// A single deserialized trace event in C-compatible form.
///
/// Inspect `tag` to determine which `payload` field to access.
/// After use, pass to [`crimp_event_free`](crate::crimp_event_free) to
/// release any heap-allocated payload data.
#[repr(C)]
pub struct CrimpEvent {
    pub tag: CrimpEventTag,
    pub payload: CrimpEventPayload,
}

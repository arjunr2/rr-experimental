//! Conversions from wasm-crimp types to C-compatible FFI types.

use std::ffi::c_char;
use std::mem::ManuallyDrop;

use wasm_crimp::component_events::BuiltinEntryEvent;
use wasm_crimp::component_events::BuiltinReturnEvent;
use wasm_crimp::{
    EventError, ExportIndex, InterfaceType, RREvent, RRFuncArgVals, RecordSettings,
    ResourceDropRet, ResultEvent,
};

use crate::types::*;

// ── Primitive conversions ───────────────────────────────────────────

impl From<Vec<u8>> for CrimpBuffer {
    fn from(v: Vec<u8>) -> Self {
        let mut v = ManuallyDrop::new(v);
        CrimpBuffer {
            ptr: v.as_mut_ptr(),
            len: v.len(),
        }
    }
}

impl CrimpBuffer {
    pub const EMPTY: Self = CrimpBuffer {
        ptr: std::ptr::null_mut(),
        len: 0,
    };

    /// Reconstruct the `Vec` and drop it, freeing the memory.
    ///
    /// # Safety
    /// Must only be called once, with the original ptr/len from a `Vec::into`.
    pub unsafe fn free(&mut self) {
        if !self.ptr.is_null() {
            unsafe {
                drop(Vec::from_raw_parts(self.ptr, self.len, self.len));
            }
            self.ptr = std::ptr::null_mut();
            self.len = 0;
        }
    }
}

impl From<String> for CrimpString {
    fn from(s: String) -> Self {
        let mut bytes = ManuallyDrop::new(s.into_bytes());
        CrimpString {
            ptr: bytes.as_mut_ptr() as *mut c_char,
            len: bytes.len(),
        }
    }
}

impl CrimpString {
    pub const EMPTY: Self = CrimpString {
        ptr: std::ptr::null_mut(),
        len: 0,
    };

    /// Reconstruct the `String` and drop it, freeing the memory.
    ///
    /// # Safety
    /// Must only be called once, with the original ptr/len from a `String::into`.
    pub unsafe fn free(&mut self) {
        if !self.ptr.is_null() {
            unsafe {
                drop(Vec::from_raw_parts(
                    self.ptr as *mut u8,
                    self.len,
                    self.len,
                ));
            }
            self.ptr = std::ptr::null_mut();
            self.len = 0;
        }
    }
}

impl From<RRFuncArgVals> for CrimpFuncArgVals {
    fn from(v: RRFuncArgVals) -> Self {
        CrimpFuncArgVals {
            bytes: v.bytes.into(),
            sizes: v.sizes.into(),
        }
    }
}

impl CrimpFuncArgVals {
    pub const EMPTY: Self = CrimpFuncArgVals {
        bytes: CrimpBuffer::EMPTY,
        sizes: CrimpBuffer::EMPTY,
    };

    /// # Safety
    /// Must only be called once per instance.
    pub unsafe fn free(&mut self) {
        unsafe {
            self.bytes.free();
            self.sizes.free();
        }
    }
}

impl From<RecordSettings> for CrimpRecordSettings {
    fn from(s: RecordSettings) -> Self {
        CrimpRecordSettings {
            add_validation: s.add_validation,
        }
    }
}

impl From<InterfaceType> for CrimpInterfaceType {
    fn from(ty: InterfaceType) -> Self {
        match ty {
            InterfaceType::Bool => CrimpInterfaceType { tag: 0, index: 0 },
            InterfaceType::S8 => CrimpInterfaceType { tag: 1, index: 0 },
            InterfaceType::U8 => CrimpInterfaceType { tag: 2, index: 0 },
            InterfaceType::S16 => CrimpInterfaceType { tag: 3, index: 0 },
            InterfaceType::U16 => CrimpInterfaceType { tag: 4, index: 0 },
            InterfaceType::S32 => CrimpInterfaceType { tag: 5, index: 0 },
            InterfaceType::U32 => CrimpInterfaceType { tag: 6, index: 0 },
            InterfaceType::S64 => CrimpInterfaceType { tag: 7, index: 0 },
            InterfaceType::U64 => CrimpInterfaceType { tag: 8, index: 0 },
            InterfaceType::Float32 => CrimpInterfaceType { tag: 9, index: 0 },
            InterfaceType::Float64 => CrimpInterfaceType {
                tag: 10,
                index: 0,
            },
            InterfaceType::Char => CrimpInterfaceType {
                tag: 11,
                index: 0,
            },
            InterfaceType::String => CrimpInterfaceType {
                tag: 12,
                index: 0,
            },
            InterfaceType::Record(i) => CrimpInterfaceType {
                tag: 13,
                index: i.as_u32(),
            },
            InterfaceType::Variant(i) => CrimpInterfaceType {
                tag: 14,
                index: i.as_u32(),
            },
            InterfaceType::List(i) => CrimpInterfaceType {
                tag: 15,
                index: i.as_u32(),
            },
            InterfaceType::Tuple(i) => CrimpInterfaceType {
                tag: 16,
                index: i.as_u32(),
            },
            InterfaceType::Flags(i) => CrimpInterfaceType {
                tag: 17,
                index: i.as_u32(),
            },
            InterfaceType::Enum(i) => CrimpInterfaceType {
                tag: 18,
                index: i.as_u32(),
            },
            InterfaceType::Option(i) => CrimpInterfaceType {
                tag: 19,
                index: i.as_u32(),
            },
            InterfaceType::Result(i) => CrimpInterfaceType {
                tag: 20,
                index: i.as_u32(),
            },
            InterfaceType::Own(i) => CrimpInterfaceType {
                tag: 21,
                index: i.as_u32(),
            },
            InterfaceType::Borrow(i) => CrimpInterfaceType {
                tag: 22,
                index: i.as_u32(),
            },
            InterfaceType::Future(i) => CrimpInterfaceType {
                tag: 23,
                index: i.as_u32(),
            },
            InterfaceType::Stream(i) => CrimpInterfaceType {
                tag: 24,
                index: i.as_u32(),
            },
            InterfaceType::ErrorContext(i) => CrimpInterfaceType {
                tag: 25,
                index: i.as_u32(),
            },
        }
    }
}

// ── Result conversions ──────────────────────────────────────────────

/// Convert a `ResultEvent<(), E>` to `CrimpResultVoid`.
fn result_void<E: EventError>(r: ResultEvent<(), E>) -> CrimpResultVoid {
    match r.ret() {
        Ok(()) => CrimpResultVoid {
            is_ok: true,
            err_msg: CrimpString::EMPTY,
        },
        Err(e) => CrimpResultVoid {
            is_ok: false,
            err_msg: CrimpString::from(e.get().clone()),
        },
    }
}

/// Convert a `ResultEvent<RRFuncArgVals, E>` to `CrimpResultFuncArgVals`.
fn result_func_arg_vals<E: EventError>(
    r: ResultEvent<RRFuncArgVals, E>,
) -> CrimpResultFuncArgVals {
    match r.ret() {
        Ok(v) => CrimpResultFuncArgVals {
            is_ok: true,
            ok_value: v.into(),
            err_msg: CrimpString::EMPTY,
        },
        Err(e) => CrimpResultFuncArgVals {
            is_ok: false,
            ok_value: CrimpFuncArgVals::EMPTY,
            err_msg: CrimpString::from(e.get().clone()),
        },
    }
}

/// Convert a `ResultEvent<usize, E>` to `CrimpResultUsize`.
fn result_usize<E: EventError>(r: ResultEvent<usize, E>) -> CrimpResultUsize {
    match r.ret() {
        Ok(v) => CrimpResultUsize {
            is_ok: true,
            ok_value: v,
            err_msg: CrimpString::EMPTY,
        },
        Err(e) => CrimpResultUsize {
            is_ok: false,
            ok_value: 0,
            err_msg: CrimpString::from(e.get().clone()),
        },
    }
}

/// Convert a `ResultEvent<u32, E>` to `CrimpResultU32`.
fn result_u32<E: EventError>(r: ResultEvent<u32, E>) -> CrimpResultU32 {
    match r.ret() {
        Ok(v) => CrimpResultU32 {
            is_ok: true,
            ok_value: v,
            err_msg: CrimpString::EMPTY,
        },
        Err(e) => CrimpResultU32 {
            is_ok: false,
            ok_value: 0,
            err_msg: CrimpString::from(e.get().clone()),
        },
    }
}

/// Convert a `ResultEvent<ResourceDropRet, E>` to `CrimpResultOptionU32`.
fn result_resource_drop<E: EventError>(
    r: ResultEvent<ResourceDropRet, E>,
) -> CrimpResultOptionU32 {
    match r.ret() {
        Ok(ResourceDropRet(opt)) => CrimpResultOptionU32 {
            is_ok: true,
            ok_has_value: opt.is_some(),
            ok_value: opt.unwrap_or(0),
            err_msg: CrimpString::EMPTY,
        },
        Err(e) => CrimpResultOptionU32 {
            is_ok: false,
            ok_has_value: false,
            ok_value: 0,
            err_msg: CrimpString::from(e.get().clone()),
        },
    }
}

/// Convert a `ResultEvent<(), E>` to unit result for builtin returns.
fn result_unit<E: EventError>(r: ResultEvent<(), E>) -> (bool, CrimpString) {
    match r.ret() {
        Ok(()) => (true, CrimpString::EMPTY),
        Err(e) => (false, CrimpString::from(e.get().clone())),
    }
}

// ── Builtin event conversions ───────────────────────────────────────

impl From<BuiltinEntryEvent> for CrimpCompBuiltinEntryEvent {
    fn from(e: BuiltinEntryEvent) -> Self {
        use wasm_crimp::component_events::*;
        match e {
            BuiltinEntryEvent::ResourceNew32(ResourceNew32EntryEvent {
                caller_instance,
                resource,
                rep,
            }) => CrimpCompBuiltinEntryEvent {
                tag: CrimpBuiltinEntryTag::ResourceNew32,
                params: [caller_instance, resource, rep],
                param_count: 3,
            },
            BuiltinEntryEvent::ResourceRep32(ResourceRep32EntryEvent {
                caller_instance,
                resource,
                idx,
            }) => CrimpCompBuiltinEntryEvent {
                tag: CrimpBuiltinEntryTag::ResourceRep32,
                params: [caller_instance, resource, idx],
                param_count: 3,
            },
            BuiltinEntryEvent::ResourceDrop(ResourceDropEntryEvent {
                caller_instance,
                resource,
                idx,
            }) => CrimpCompBuiltinEntryEvent {
                tag: CrimpBuiltinEntryTag::ResourceDrop,
                params: [caller_instance, resource, idx],
                param_count: 3,
            },
            BuiltinEntryEvent::ResourceTransferOwn(ResourceTransferOwnEntryEvent {
                src_idx,
                src_table,
                dst_table,
            }) => CrimpCompBuiltinEntryEvent {
                tag: CrimpBuiltinEntryTag::ResourceTransferOwn,
                params: [src_idx, src_table, dst_table],
                param_count: 3,
            },
            BuiltinEntryEvent::ResourceTransferBorrow(ResourceTransferBorrowEntryEvent {
                src_idx,
                src_table,
                dst_table,
            }) => CrimpCompBuiltinEntryEvent {
                tag: CrimpBuiltinEntryTag::ResourceTransferBorrow,
                params: [src_idx, src_table, dst_table],
                param_count: 3,
            },
            BuiltinEntryEvent::ResourceEnterCall(ResourceEnterCallEntryEvent {}) => {
                CrimpCompBuiltinEntryEvent {
                    tag: CrimpBuiltinEntryTag::ResourceEnterCall,
                    params: [0; 3],
                    param_count: 0,
                }
            }
            BuiltinEntryEvent::ResourceExitCall(ResourceExitCallEntryEvent {}) => {
                CrimpCompBuiltinEntryEvent {
                    tag: CrimpBuiltinEntryTag::ResourceExitCall,
                    params: [0; 3],
                    param_count: 0,
                }
            }
        }
    }
}

impl From<BuiltinReturnEvent> for CrimpCompBuiltinReturnEvent {
    fn from(e: BuiltinReturnEvent) -> Self {
        use wasm_crimp::component_events::*;
        match e {
            BuiltinReturnEvent::ResourceNew32(ResourceNew32ReturnEvent(r)) => {
                let cr = result_u32(r);
                CrimpCompBuiltinReturnEvent {
                    tag: CrimpBuiltinReturnTag::ResourceNew32,
                    is_ok: cr.is_ok,
                    ok_u32: cr.ok_value,
                    ok_has_value: false,
                    err_msg: cr.err_msg,
                }
            }
            BuiltinReturnEvent::ResourceRep32(ResourceRep32ReturnEvent(r)) => {
                let cr = result_u32(r);
                CrimpCompBuiltinReturnEvent {
                    tag: CrimpBuiltinReturnTag::ResourceRep32,
                    is_ok: cr.is_ok,
                    ok_u32: cr.ok_value,
                    ok_has_value: false,
                    err_msg: cr.err_msg,
                }
            }
            BuiltinReturnEvent::ResourceDrop(ResourceDropReturnEvent(r)) => {
                let cr = result_resource_drop(r);
                CrimpCompBuiltinReturnEvent {
                    tag: CrimpBuiltinReturnTag::ResourceDrop,
                    is_ok: cr.is_ok,
                    ok_u32: cr.ok_value,
                    ok_has_value: cr.ok_has_value,
                    err_msg: cr.err_msg,
                }
            }
            BuiltinReturnEvent::ResourceTransferOwn(ResourceTransferOwnReturnEvent(r)) => {
                let cr = result_u32(r);
                CrimpCompBuiltinReturnEvent {
                    tag: CrimpBuiltinReturnTag::ResourceTransferOwn,
                    is_ok: cr.is_ok,
                    ok_u32: cr.ok_value,
                    ok_has_value: false,
                    err_msg: cr.err_msg,
                }
            }
            BuiltinReturnEvent::ResourceTransferBorrow(ResourceTransferBorrowReturnEvent(r)) => {
                let cr = result_u32(r);
                CrimpCompBuiltinReturnEvent {
                    tag: CrimpBuiltinReturnTag::ResourceTransferBorrow,
                    is_ok: cr.is_ok,
                    ok_u32: cr.ok_value,
                    ok_has_value: false,
                    err_msg: cr.err_msg,
                }
            }
            BuiltinReturnEvent::ResourceExitCall(ResourceExitCallReturnEvent(r)) => {
                let (is_ok, err_msg) = result_unit(r);
                CrimpCompBuiltinReturnEvent {
                    tag: CrimpBuiltinReturnTag::ResourceExitCall,
                    is_ok,
                    ok_u32: 0,
                    ok_has_value: false,
                    err_msg,
                }
            }
        }
    }
}

// ── Top-level RREvent → CrimpEvent conversion ──────────────────────

/// Helper to wrap a payload field in `ManuallyDrop` and build a `CrimpEvent`.
macro_rules! crimp_event {
    ($tag:ident, $field:ident, $value:expr) => {
        CrimpEvent {
            tag: CrimpEventTag::$tag,
            payload: CrimpEventPayload {
                $field: ManuallyDrop::new($value),
            },
        }
    };
}

fn export_index_to_u32(idx: ExportIndex) -> u32 {
    idx.as_u32()
}

impl From<RREvent> for CrimpEvent {
    fn from(event: RREvent) -> Self {
        use wasm_crimp::{common_events, component_events, core_events};

        match event {
            RREvent::Nop => CrimpEvent {
                tag: CrimpEventTag::Nop,
                payload: CrimpEventPayload { none: () },
            },
            RREvent::Eof => CrimpEvent {
                tag: CrimpEventTag::Eof,
                payload: CrimpEventPayload { none: () },
            },

            RREvent::TraceSignature(common_events::TraceSignatureEvent {
                checksum,
                settings,
            }) => crimp_event!(
                TraceSignature,
                trace_signature,
                CrimpTraceSignatureEvent {
                    checksum: checksum.into(),
                    settings: settings.into(),
                }
            ),

            RREvent::CustomMessage(common_events::CustomMessageEvent(msg)) => crimp_event!(
                CustomMessage,
                custom_message,
                CrimpCustomMessageEvent {
                    message: msg.into(),
                }
            ),

            RREvent::HostFuncReturn(common_events::HostFuncReturnEvent { args }) => crimp_event!(
                HostFuncReturn,
                host_func_return,
                CrimpHostFuncReturnEvent { args: args.into() }
            ),

            RREvent::HostFuncEntry(common_events::HostFuncEntryEvent { args }) => crimp_event!(
                HostFuncEntry,
                host_func_entry,
                CrimpHostFuncEntryEvent { args: args.into() }
            ),

            RREvent::WasmFuncReturn(common_events::WasmFuncReturnEvent(r)) => crimp_event!(
                WasmFuncReturn,
                wasm_func_return,
                CrimpWasmFuncReturnEvent {
                    result: result_func_arg_vals(r),
                }
            ),

            RREvent::CoreWasmInstantiation(core_events::InstantiationEvent {
                module,
                instance,
            }) => crimp_event!(
                CoreWasmInstantiation,
                core_instantiation,
                CrimpCoreInstantiationEvent {
                    module: *module,
                    instance: instance.0,
                }
            ),

            RREvent::CoreWasmFuncEntry(core_events::WasmFuncEntryEvent {
                instance,
                func_index,
                args,
            }) => crimp_event!(
                CoreWasmFuncEntry,
                core_wasm_func_entry,
                CrimpCoreWasmFuncEntryEvent {
                    instance: instance.0,
                    func_index: func_index.0,
                    args: args.into(),
                }
            ),

            RREvent::ComponentWasmFuncBegin(component_events::WasmFuncBeginEvent {
                instance,
                func_index,
            }) => crimp_event!(
                ComponentWasmFuncBegin,
                comp_wasm_func_begin,
                CrimpCompWasmFuncBeginEvent {
                    instance: instance.0,
                    func_index: export_index_to_u32(func_index),
                }
            ),

            RREvent::ComponentWasmFuncEntry(component_events::WasmFuncEntryEvent { args }) => {
                crimp_event!(
                    ComponentWasmFuncEntry,
                    comp_wasm_func_entry,
                    CrimpCompWasmFuncEntryEvent { args: args.into() }
                )
            }

            RREvent::ComponentInstantiation(component_events::InstantiationEvent {
                component,
                instance,
            }) => crimp_event!(
                ComponentInstantiation,
                comp_instantiation,
                CrimpCompInstantiationEvent {
                    component: *component,
                    instance: instance.0,
                }
            ),

            RREvent::ComponentReallocEntry(component_events::ReallocEntryEvent {
                old_addr,
                old_size,
                old_align,
                new_size,
            }) => crimp_event!(
                ComponentReallocEntry,
                comp_realloc_entry,
                CrimpCompReallocEntryEvent {
                    old_addr,
                    old_size,
                    old_align,
                    new_size,
                }
            ),

            RREvent::ComponentLowerFlatReturn(component_events::LowerFlatReturnEvent(r)) => {
                crimp_event!(
                    ComponentLowerFlatReturn,
                    comp_lower_flat_return,
                    CrimpCompLowerFlatReturnEvent {
                        result: result_void(r),
                    }
                )
            }

            RREvent::ComponentLowerMemoryReturn(component_events::LowerMemoryReturnEvent(r)) => {
                crimp_event!(
                    ComponentLowerMemoryReturn,
                    comp_lower_memory_return,
                    CrimpCompLowerMemoryReturnEvent {
                        result: result_void(r),
                    }
                )
            }

            RREvent::ComponentMemorySliceWrite(component_events::MemorySliceWriteEvent {
                offset,
                bytes,
            }) => crimp_event!(
                ComponentMemorySliceWrite,
                comp_memory_slice_write,
                CrimpCompMemorySliceWriteEvent {
                    offset,
                    bytes: bytes.into(),
                }
            ),

            RREvent::ComponentBuiltinReturn(e) => crimp_event!(
                ComponentBuiltinReturn,
                comp_builtin_return,
                e.into()
            ),

            RREvent::ComponentPostReturn(component_events::PostReturnEvent {
                instance,
                func_index,
            }) => crimp_event!(
                ComponentPostReturn,
                comp_post_return,
                CrimpCompPostReturnEvent {
                    instance: instance.0,
                    func_index: export_index_to_u32(func_index),
                }
            ),

            RREvent::ComponentReallocReturn(component_events::ReallocReturnEvent(r)) => {
                crimp_event!(
                    ComponentReallocReturn,
                    comp_realloc_return,
                    CrimpCompReallocReturnEvent {
                        result: result_usize(r),
                    }
                )
            }

            RREvent::ComponentLowerFlatEntry(component_events::LowerFlatEntryEvent { ty }) => {
                crimp_event!(
                    ComponentLowerFlatEntry,
                    comp_lower_flat_entry,
                    CrimpCompLowerFlatEntryEvent { ty: ty.into() }
                )
            }

            RREvent::ComponentLowerMemoryEntry(component_events::LowerMemoryEntryEvent {
                ty,
                offset,
            }) => crimp_event!(
                ComponentLowerMemoryEntry,
                comp_lower_memory_entry,
                CrimpCompLowerMemoryEntryEvent {
                    ty: ty.into(),
                    offset,
                }
            ),

            RREvent::ComponentBuiltinEntry(e) => {
                crimp_event!(ComponentBuiltinEntry, comp_builtin_entry, e.into())
            }
        }
    }
}

// ── CrimpEvent::free ────────────────────────────────────────────────

impl CrimpEvent {
    /// Free any heap-allocated data within this event's payload.
    ///
    /// # Safety
    /// Must only be called once per event. The `tag` must match the
    /// active payload variant.
    pub unsafe fn free(&mut self) {
        unsafe {
            match self.tag {
                CrimpEventTag::Eof | CrimpEventTag::Nop => {}

                CrimpEventTag::Error => {
                    ManuallyDrop::drop(&mut self.payload.error_msg);
                }

                CrimpEventTag::TraceSignature => {
                    let sig = &mut *self.payload.trace_signature;
                    sig.checksum.free();
                }

                CrimpEventTag::CustomMessage => {
                    let msg = &mut *self.payload.custom_message;
                    msg.message.free();
                }

                CrimpEventTag::HostFuncReturn => {
                    let e = &mut *self.payload.host_func_return;
                    e.args.free();
                }

                CrimpEventTag::HostFuncEntry => {
                    let e = &mut *self.payload.host_func_entry;
                    e.args.free();
                }

                CrimpEventTag::WasmFuncReturn => {
                    let e = &mut *self.payload.wasm_func_return;
                    e.result.ok_value.free();
                    e.result.err_msg.free();
                }

                CrimpEventTag::CoreWasmInstantiation => {
                    // All fixed-size fields, nothing to free
                }

                CrimpEventTag::CoreWasmFuncEntry => {
                    let e = &mut *self.payload.core_wasm_func_entry;
                    e.args.free();
                }

                CrimpEventTag::ComponentWasmFuncBegin => {
                    // All fixed-size fields
                }

                CrimpEventTag::ComponentWasmFuncEntry => {
                    let e = &mut *self.payload.comp_wasm_func_entry;
                    e.args.free();
                }

                CrimpEventTag::ComponentInstantiation => {
                    // All fixed-size fields
                }

                CrimpEventTag::ComponentReallocEntry => {
                    // All fixed-size fields
                }

                CrimpEventTag::ComponentLowerFlatReturn => {
                    let e = &mut *self.payload.comp_lower_flat_return;
                    e.result.err_msg.free();
                }

                CrimpEventTag::ComponentLowerMemoryReturn => {
                    let e = &mut *self.payload.comp_lower_memory_return;
                    e.result.err_msg.free();
                }

                CrimpEventTag::ComponentMemorySliceWrite => {
                    let e = &mut *self.payload.comp_memory_slice_write;
                    e.bytes.free();
                }

                CrimpEventTag::ComponentBuiltinReturn => {
                    let e = &mut *self.payload.comp_builtin_return;
                    e.err_msg.free();
                }

                CrimpEventTag::ComponentPostReturn => {
                    // All fixed-size fields
                }

                CrimpEventTag::ComponentReallocReturn => {
                    let e = &mut *self.payload.comp_realloc_return;
                    e.result.err_msg.free();
                }

                CrimpEventTag::ComponentLowerFlatEntry => {
                    // All fixed-size fields
                }

                CrimpEventTag::ComponentLowerMemoryEntry => {
                    // All fixed-size fields
                }

                CrimpEventTag::ComponentBuiltinEntry => {
                    // All fixed-size fields
                }
            }
        }
    }
}

//! Import helpers that binds to the glue logic, allowing orchestrating the replay

type ArgBytesPtr = *mut u8;
type ArgSizesPtr = *mut u8;
type ArgSizesLen = u32;
type ArgBytesLen = u32;

/// Type for glue and driver passing func arg vals to and from each other (e.g. for host func params or wasm function returns)
#[repr(C)]
pub struct RRFuncArgValsFFI {
    pub bytes_ptr: ArgBytesPtr,
    pub sizes_ptr: ArgSizesPtr,
    // The lengths below are only used for Wasm Function returns
    pub bytes_len: ArgBytesLen,
    pub sizes_len: ArgSizesLen,
}

#[link(wasm_import_module = "crimp_glue")]
unsafe extern "C" {
    /// Get the checksum of the currently instantiated component/module being driven.
    ///
    /// The caller is expected to populate the buffer with 32 bytes to write the checksum into.
    /// This is to avoid dynamic memory allocation in the driver.
    pub fn get_sha256_checksum(checksum_buf: *mut u8);

    ///// A dispatch method that calls the appropriate realloc for the given export's lowering.
    //pub fn dispatch_realloc(
    //    export_index: u32,
    //    old_addr: u32,
    //    old_size: u32,
    //    old_align: u32,
    //    new_size: u32,
    //) -> u32;

    ///// A dispatch method that calls the appropriate memory write for the given export's lowering.
    //pub fn dispatch_memory_write(
    //    export_index: u32,
    //    offset: u32,
    //    bytes_ptr: *const u8,
    //    num_bytes: u32,
    //);

    /// A dispatch method that calls the appropriate core function post-lowering function for the given export.
    ///
    /// The glue logic already has the signature, so it encodes the data in the return buffer, and gives the host back
    /// the sizes.
    pub fn dispatch_core_func(
        export_index: u32,
        args: *const u8,
        return_bytes_len: *mut u32,
        return_sizes_len: *mut u32,
    );
}

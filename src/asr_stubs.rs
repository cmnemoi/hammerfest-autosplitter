//! Link ballast for `cargo test`.
//!
//! `asr` declares the LiveSplit runtime ABI as one `extern "C"` block, in
//! `src/runtime/sys.rs`. The runtime provides those symbols inside the
//! WebAssembly sandbox. Nothing provides them on a development machine, so
//! `cargo test -p hammerfest-autosplitter` fails to link.
//!
//! Twenty-eight survive once the reader reads through
//! [`crate::avm1::Memory`]. Twenty-four are `libasr`'s own object files, for
//! settings and timer APIs this project never calls. `/OPT:REF` keeps them
//! because they sit in codegen units it retains, and no design on our side
//! removes them.
//!
//! Those twenty-four carry no behaviour at all. The reader is served a
//! `Memory` of its own, so nothing here decides what a read means for it.
//! `unimplemented!()` is the right body, and a firing stub is a message: the
//! reader grew a tie to the runtime that the trait does not cover.
//!
//! Four have bodies. `runtime_print_message` returns, because the reader
//! prints while it searches. `process_attach_by_pid`, `process_detach` and
//! `process_read` let `crate::memory_contract` build an `asr::Process` and
//! serve it bytes, so that the production adapter answers the same contract
//! as the test heap.
//!
//! That run proves the adapter forwards correctly. It cannot prove what
//! LiveSplit does, because the bytes still come from this file. See
//! `docs/internals/testing-the-memory-reader.md`.

extern crate std;

use std::{
    num::NonZeroU64,
    sync::{Mutex, MutexGuard},
    vec::Vec,
};

/// How the host answers a read.
pub enum Host {
    /// Serves the ranges, and refuses whatever they do not hold whole.
    Honest,
    /// Writes one byte into the buffer, then fails.
    ///
    /// A runtime is allowed to do this. `sys.rs` promises nothing about the
    /// buffer when `process_read` returns `false`.
    DirtiesThenFails,
}

struct Served {
    regions: Vec<(u64, Vec<u8>)>,
    host: Host,
}

static SERVED: Mutex<Option<Served>> = Mutex::new(None);

/// Serialises the contract tests.
///
/// A `extern "C"` stub receives no context, so what it serves is global. Only
/// the adapter's own contract tests take this lock. The reader's tests use a
/// `Memory` of their own and never come here.
//
// ponytail: one host at a time. It carries four tests, so the serialisation
// costs nothing worth measuring.
static LOCK: Mutex<()> = Mutex::new(());

/// Holds the bytes in place, and the other contract tests out.
pub struct Installed(#[allow(dead_code)] MutexGuard<'static, ()>);

impl Drop for Installed {
    fn drop(&mut self) {
        *SERVED.lock().unwrap_or_else(|e| e.into_inner()) = None;
    }
}

/// Serves `regions` through `asr::Process` until the return value is dropped.
pub fn serve(regions: Vec<(u64, Vec<u8>)>, host: Host) -> Installed {
    let guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    *SERVED.lock().unwrap_or_else(|e| e.into_inner()) = Some(Served { regions, host });
    Installed(guard)
}

/// The handle every stub answers to. Which process it is never matters.
const HANDLE: u64 = 1;

#[no_mangle]
extern "C" fn process_attach_by_pid(_pid: u64) -> Option<NonZeroU64> {
    NonZeroU64::new(HANDLE)
}

#[no_mangle]
extern "C" fn process_detach(_process: NonZeroU64) {}

#[no_mangle]
extern "C" fn process_read(
    _process: NonZeroU64,
    address: u64,
    buf_ptr: *mut u8,
    buf_len: usize,
) -> bool {
    let guard = SERVED.lock().unwrap_or_else(|e| e.into_inner());
    let Some(served) = guard.as_ref() else {
        return false;
    };
    if matches!(served.host, Host::DirtiesThenFails) {
        // SAFETY: the caller owns `buf_len` bytes at `buf_ptr`, and this
        // writes one of them.
        if buf_len > 0 {
            unsafe { buf_ptr.write(0xAA) };
        }
        return false;
    }
    let Some((base, bytes)) = served
        .regions
        .iter()
        .find(|(base, bytes)| address >= *base && address - *base < bytes.len() as u64)
    else {
        return false;
    };
    let offset = (address - base) as usize;
    let Some(end) = offset.checked_add(buf_len) else {
        return false;
    };
    let Some(source) = bytes.get(offset..end) else {
        return false;
    };
    // SAFETY: the caller owns `buf_len` bytes at `buf_ptr`, and `source` holds
    // exactly that many.
    unsafe { core::ptr::copy_nonoverlapping(source.as_ptr(), buf_ptr, buf_len) };
    true
}

/// Defines a symbol and nothing else.
///
/// The signature is deliberately wrong and the linker never reads it. The body
/// panics before it can return, so no caller ever sees the mismatch.
macro_rules! link_only {
    ($($name:ident,)*) => {
        $(
            #[no_mangle]
            extern "C" fn $name() {
                unimplemented!(concat!(stringify!($name), ": outside the reader"));
            }
        )*
    };
}

#[no_mangle]
extern "C" fn runtime_print_message(text_ptr: *const u8, text_len: usize) {
    let text = unsafe { core::slice::from_raw_parts(text_ptr, text_len) };
    std::eprintln!("[asr] {}", std::string::String::from_utf8_lossy(text));
}

link_only! {
    process_list_by_name,
    setting_value_free,
    setting_value_get_bool,
    setting_value_get_f64,
    setting_value_get_i64,
    setting_value_get_list,
    setting_value_get_map,
    setting_value_get_string,
    setting_value_get_type,
    settings_list_free,
    settings_list_get,
    settings_list_len,
    settings_map_free,
    settings_map_get,
    settings_map_get_key_by_index,
    settings_map_get_value_by_index,
    settings_map_len,
    settings_map_load,
    timer_current_split_index,
    timer_segment_splitted,
    user_settings_add_file_select,
    user_settings_add_file_select_mime_filter,
    user_settings_add_file_select_name_filter,
    user_settings_add_text_input,
}

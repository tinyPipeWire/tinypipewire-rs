use std::ffi::{c_void, CStr, CString};
use std::panic::{catch_unwind, AssertUnwindSafe};

use crate::error::{Error, Result};

/// Runs a callback trampoline, swallowing a panic rather than letting it
/// unwind into C.
pub(crate) fn guard(body: impl FnOnce()) {
    let _ = catch_unwind(AssertUnwindSafe(body));
}

/// Borrows a Rust string as a C string for the duration of `body`.
pub(crate) fn with_cstr<T>(s: &str, body: impl FnOnce(&CStr) -> T) -> Result<T> {
    let owned = CString::new(s).map_err(|_| Error::InvalidString)?;
    Ok(body(&owned))
}

/// Recovers the boxed state a callback was registered with.
///
/// # Safety
/// `user_data` must be the pointer handed to the C library when the owning
/// handle was created, and its state must still be alive.
pub(crate) unsafe fn state<'a, S>(user_data: *mut c_void) -> Option<&'a S> {
    user_data.cast::<S>().as_ref()
}

/// Growth ceiling for the list queries below, so a device that keeps
/// reporting more entries cannot spin here forever.
const LIST_LIMIT: usize = 4096;

/// Drives one of the C library's "fill up to `len`, return how many exist"
/// queries, growing the buffer once if the first guess was too small.
///
/// # Safety
/// `fill` must write no more than `len` entries to the pointer it is given.
pub(crate) unsafe fn collect_list<R: Copy, T>(
    initial: usize,
    mut fill: impl FnMut(*mut R, usize) -> usize,
    map: impl Fn(&R) -> T,
) -> Vec<T> {
    let mut cap = initial.max(1);
    loop {
        let mut buf: Vec<R> = vec![std::mem::zeroed(); cap];
        let found = fill(buf.as_mut_ptr(), cap);
        if found <= cap || cap >= LIST_LIMIT {
            return buf[..found.min(cap)].iter().map(&map).collect();
        }
        cap = found.min(LIST_LIMIT);
    }
}

// Copyright (c) 2026 Parsa Bagheri
//
// This software is released under the Apache-2.0 License.

//! The boundary machinery every `extern "C"` function goes through: errors, the
//! per-thread last-error message, panic containment, and the version and result
//! name functions.

use std::cell::RefCell;
use std::ffi::{c_char, CStr, CString};
use std::panic::{catch_unwind, AssertUnwindSafe};

use crate::types::*;

/// An error to report across the boundary: a result code and a message for
/// `kaman_last_error_message`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Error {
    pub(crate) code: KamanResult,
    pub(crate) message: String,
}

impl Error {
    pub(crate) fn new(code: KamanResult, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }

    pub(crate) fn invalid_argument(message: impl Into<String>) -> Self {
        Self::new(KAMAN_ERROR_INVALID_ARGUMENT, message)
    }

    pub(crate) fn invalid_handle(what: &str) -> Self {
        Self::new(
            KAMAN_ERROR_INVALID_HANDLE,
            format!("{what} is not a live handle of this context"),
        )
    }

    pub(crate) fn wrong_state(message: impl Into<String>) -> Self {
        Self::new(KAMAN_ERROR_WRONG_STATE, message)
    }

    pub(crate) fn null(what: &str) -> Self {
        Self::invalid_argument(format!("{what} is NULL"))
    }
}

/// The result of a boundary function's body: a non-error code (`KAMAN_OK`,
/// `KAMAN_FRAME_SKIPPED`, `KAMAN_NOT_READY`) or an [`Error`].
pub(crate) type Outcome = Result<KamanResult, Error>;

thread_local! {
    static LAST_ERROR: RefCell<CString> = RefCell::new(CString::default());
}

fn set_last_error(message: &str) {
    // Interior NULs cannot appear in a C string; replace them.
    let message = CString::new(message.replace('\0', " ")).unwrap_or_default();
    LAST_ERROR.with(|slot| *slot.borrow_mut() = message);
}

fn panic_message(payload: &(dyn std::any::Any + Send)) -> &str {
    payload
        .downcast_ref::<&str>()
        .copied()
        .or_else(|| payload.downcast_ref::<String>().map(String::as_str))
        .unwrap_or("unknown panic")
}

/// Run a boundary function's body: map errors to their code (recording the
/// message), and turn a panic into `KAMAN_ERROR_INTERNAL` so it never unwinds
/// into C.
pub(crate) fn call(body: impl FnOnce() -> Outcome) -> KamanResult {
    match catch_unwind(AssertUnwindSafe(body)) {
        Ok(Ok(code)) => code,
        Ok(Err(error)) => {
            set_last_error(&error.message);
            error.code
        }
        Err(payload) => {
            set_last_error(&format!(
                "internal error (a bug in KamanEngine): {}",
                panic_message(payload.as_ref())
            ));
            KAMAN_ERROR_INTERNAL
        }
    }
}

/// Like [`call`] for functions with no error channel: a panic yields `fallback`.
pub(crate) fn call_or<T>(fallback: T, body: impl FnOnce() -> T) -> T {
    catch_unwind(AssertUnwindSafe(body)).unwrap_or(fallback)
}

/// Borrow a required input pointer.
///
/// # Safety
/// `ptr` must be NULL or valid for reads of `T` for the call.
pub(crate) unsafe fn input<'a, T>(ptr: *const T, what: &str) -> Result<&'a T, Error> {
    ptr.as_ref().ok_or_else(|| Error::null(what))
}

/// Borrow a required output pointer.
///
/// # Safety
/// `ptr` must be NULL or valid for writes of `T` for the call.
pub(crate) unsafe fn output<'a, T>(ptr: *mut T, what: &str) -> Result<&'a mut T, Error> {
    ptr.as_mut().ok_or_else(|| Error::null(what))
}

/// Read a NUL-terminated UTF-8 string argument.
///
/// # Safety
/// `ptr` must be NULL or point to a NUL-terminated string valid for the call.
pub(crate) unsafe fn utf8<'a>(ptr: *const c_char, what: &str) -> Result<&'a str, Error> {
    if ptr.is_null() {
        return Err(Error::null(what));
    }
    CStr::from_ptr(ptr)
        .to_str()
        .map_err(|_| Error::invalid_argument(format!("{what} is not valid UTF-8")))
}

/// `KAMAN_API_VERSION` as compiled into the library.
#[no_mangle]
pub extern "C" fn kaman_api_version() -> u32 {
    KAMAN_API_VERSION
}

/// The library's release, e.g. `"0.1.0-beta.1"`. Static; thread-safe.
#[no_mangle]
pub extern "C" fn kaman_version_string() -> *const c_char {
    concat!(env!("CARGO_PKG_VERSION"), "\0").as_ptr().cast()
}

/// The name of a result constant, e.g. `"KAMAN_ERROR_BUSY"`, or
/// `"KAMAN_UNKNOWN_RESULT"`. Static; thread-safe.
#[no_mangle]
pub extern "C" fn kaman_result_name(result: KamanResult) -> *const c_char {
    let name: &CStr = match result {
        KAMAN_OK => c"KAMAN_OK",
        KAMAN_FRAME_SKIPPED => c"KAMAN_FRAME_SKIPPED",
        KAMAN_NOT_READY => c"KAMAN_NOT_READY",
        KAMAN_ERROR_INVALID_ARGUMENT => c"KAMAN_ERROR_INVALID_ARGUMENT",
        KAMAN_ERROR_INVALID_HANDLE => c"KAMAN_ERROR_INVALID_HANDLE",
        KAMAN_ERROR_WRONG_STATE => c"KAMAN_ERROR_WRONG_STATE",
        KAMAN_ERROR_BUSY => c"KAMAN_ERROR_BUSY",
        KAMAN_ERROR_NO_DEVICE => c"KAMAN_ERROR_NO_DEVICE",
        KAMAN_ERROR_UNSUPPORTED => c"KAMAN_ERROR_UNSUPPORTED",
        KAMAN_ERROR_IO => c"KAMAN_ERROR_IO",
        KAMAN_ERROR_PARSE => c"KAMAN_ERROR_PARSE",
        KAMAN_ERROR_LIMIT_EXCEEDED => c"KAMAN_ERROR_LIMIT_EXCEEDED",
        KAMAN_ERROR_INTERNAL => c"KAMAN_ERROR_INTERNAL",
        _ => c"KAMAN_UNKNOWN_RESULT",
    };
    name.as_ptr()
}

/// This thread's last error description: valid until the next failing call on
/// this thread, never NULL (empty when there is none). Thread-safe.
#[no_mangle]
pub extern "C" fn kaman_last_error_message() -> *const c_char {
    // The CString lives in this thread's slot until the next error replaces it,
    // so its buffer outlives this call as documented.
    LAST_ERROR.with(|slot| slot.borrow().as_ptr())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn last_error() -> String {
        // SAFETY: kaman_last_error_message never returns NULL and the buffer is
        // valid until the next error on this thread.
        unsafe { CStr::from_ptr(kaman_last_error_message()) }
            .to_string_lossy()
            .into_owned()
    }

    #[test]
    fn errors_set_the_last_error_and_return_their_code() {
        let code = call(|| Err(Error::invalid_argument("bad width")));
        assert_eq!(code, KAMAN_ERROR_INVALID_ARGUMENT);
        assert_eq!(last_error(), "bad width");
        // A success leaves the last error in place.
        assert_eq!(call(|| Ok(KAMAN_OK)), KAMAN_OK);
        assert_eq!(last_error(), "bad width");
    }

    #[test]
    fn a_panic_becomes_internal_and_never_unwinds() {
        let code = call(|| panic!("boom"));
        assert_eq!(code, KAMAN_ERROR_INTERNAL);
        assert!(last_error().contains("boom"), "{}", last_error());
        assert_eq!(call_or(7, || panic!("again")), 7);
    }

    #[test]
    fn messages_with_interior_nuls_survive() {
        let _ = call(|| Err(Error::invalid_argument("a\0b")));
        assert_eq!(last_error(), "a b");
    }

    #[test]
    fn every_result_has_a_name() {
        // SAFETY: the names are static NUL-terminated strings.
        let name = |r| {
            unsafe { CStr::from_ptr(kaman_result_name(r)) }
                .to_str()
                .unwrap()
        };
        assert_eq!(name(KAMAN_OK), "KAMAN_OK");
        assert_eq!(name(KAMAN_ERROR_BUSY), "KAMAN_ERROR_BUSY");
        assert_eq!(name(12345), "KAMAN_UNKNOWN_RESULT");
        for code in -10..=2 {
            assert_ne!(name(code), "KAMAN_UNKNOWN_RESULT", "{code} has no name");
        }
    }
}

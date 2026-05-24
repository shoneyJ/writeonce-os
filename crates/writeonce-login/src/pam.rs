//! Hand-rolled FFI to libpam, plus a safe `Session` wrapper.
//!
//! We don't use the `pam-sys` crate so that the dependency surface
//! stays bounded to libc + serde + toml. The PAM API is small enough
//! to declare directly.
//!
//! Conversation model: PAM calls our `conversation` callback whenever
//! it needs to ask the user something (typically the password). We
//! thread a `Conversation` trait object through `appdata_ptr`; the
//! callback dispatches each `pam_message` to that trait's
//! `prompt_echo_off()` / `prompt_echo_on()` / `info()` / `error()`
//! method.

#![allow(non_snake_case, non_camel_case_types, dead_code)]

use std::ffi::{CStr, CString, c_int, c_void};
use std::ptr;

// --- Opaque PAM handle ------------------------------------------------------

pub enum pam_handle {}    // opaque

// --- C structs ---------------------------------------------------------------

#[repr(C)]
struct pam_message {
    msg_style: c_int,
    msg:       *const libc::c_char,
}

#[repr(C)]
struct pam_response {
    resp:         *mut libc::c_char,
    resp_retcode: c_int,
}

#[repr(C)]
struct pam_conv {
    conv:         extern "C" fn(
                      c_int,
                      *mut *const pam_message,
                      *mut *mut pam_response,
                      *mut c_void,
                  ) -> c_int,
    appdata_ptr:  *mut c_void,
}

// --- Constants from <security/_pam_types.h> ---------------------------------

const PAM_SUCCESS: c_int = 0;

const PAM_PROMPT_ECHO_OFF: c_int = 1;
const PAM_PROMPT_ECHO_ON:  c_int = 2;
const PAM_ERROR_MSG:       c_int = 3;
const PAM_TEXT_INFO:       c_int = 4;

pub const PAM_USER: c_int = 2;

pub const PAM_SILENT:           c_int = 0x8000;
pub const PAM_ESTABLISH_CRED:   c_int = 0x0002;
pub const PAM_DELETE_CRED:      c_int = 0x0004;
pub const PAM_REINITIALIZE_CRED: c_int = 0x0008;
pub const PAM_REFRESH_CRED:     c_int = 0x0010;

const PAM_CONV_ERR: c_int = 19;
const PAM_BUF_ERR:  c_int = 5;

// --- extern "C" declarations ------------------------------------------------

#[link(name = "pam")]
extern "C" {
    fn pam_start(
        service_name: *const libc::c_char,
        user:         *const libc::c_char,
        conv:         *const pam_conv,
        pamh:         *mut *mut pam_handle,
    ) -> c_int;

    fn pam_end(pamh: *mut pam_handle, pam_status: c_int) -> c_int;

    fn pam_authenticate(pamh: *mut pam_handle, flags: c_int) -> c_int;
    fn pam_acct_mgmt   (pamh: *mut pam_handle, flags: c_int) -> c_int;
    fn pam_setcred     (pamh: *mut pam_handle, flags: c_int) -> c_int;
    fn pam_open_session(pamh: *mut pam_handle, flags: c_int) -> c_int;
    fn pam_close_session(pamh: *mut pam_handle, flags: c_int) -> c_int;

    fn pam_get_item(
        pamh: *const pam_handle,
        item_type: c_int,
        item: *mut *const c_void,
    ) -> c_int;

    fn pam_strerror(pamh: *const pam_handle, errnum: c_int) -> *const libc::c_char;
}

// --- Conversation trait -----------------------------------------------------

/// What the supervisor uses to talk to PAM. Implementors render each
/// prompt on the tty and return the user's response.
pub trait Conversation {
    fn prompt_echo_off(&mut self, msg: &str) -> Option<String>;
    fn prompt_echo_on (&mut self, msg: &str) -> Option<String>;
    fn info           (&mut self, msg: &str);
    fn error          (&mut self, msg: &str);
}

// --- The C-side conversation callback ---------------------------------------

extern "C" fn conv_trampoline(
    num_msg:     c_int,
    msg:         *mut *const pam_message,
    resp_out:    *mut *mut pam_response,
    appdata_ptr: *mut c_void,
) -> c_int {
    // Safety: PAM guarantees msg is a valid array of num_msg pointers.
    if num_msg <= 0 || msg.is_null() || resp_out.is_null() || appdata_ptr.is_null() {
        return PAM_CONV_ERR;
    }
    let num = num_msg as usize;

    // Allocate the response array via libc::calloc so PAM can free() it.
    let resp_array = unsafe { libc::calloc(num, std::mem::size_of::<pam_response>()) } as *mut pam_response;
    if resp_array.is_null() { return PAM_BUF_ERR; }

    // SAFETY: appdata_ptr was set by Session::authenticate to
    // &mut *Box<dyn Conversation> via leak/raw.
    let conv: &mut Box<dyn Conversation> = unsafe {
        &mut *(appdata_ptr as *mut Box<dyn Conversation>)
    };

    for i in 0..num {
        let m: *const pam_message = unsafe { *msg.add(i) };
        if m.is_null() { continue; }
        let style    = unsafe { (*m).msg_style };
        let raw_text = unsafe { (*m).msg };
        let text = if raw_text.is_null() {
            ""
        } else {
            unsafe { CStr::from_ptr(raw_text) }.to_str().unwrap_or("")
        };

        let answer: Option<String> = match style {
            PAM_PROMPT_ECHO_OFF => conv.prompt_echo_off(text),
            PAM_PROMPT_ECHO_ON  => conv.prompt_echo_on (text),
            PAM_TEXT_INFO       => { conv.info (text); None }
            PAM_ERROR_MSG       => { conv.error(text); None }
            _ => None,
        };

        if let Some(s) = answer {
            // Allocate the response string with libc::strdup-equivalent
            // so PAM can free(3) it.
            let cstr = CString::new(s).unwrap_or_default();
            let n = cstr.as_bytes_with_nul().len();
            let buf = unsafe { libc::malloc(n) } as *mut libc::c_char;
            if buf.is_null() {
                // Free anything we already allocated and bail.
                unsafe { libc::free(resp_array as *mut c_void) };
                return PAM_BUF_ERR;
            }
            unsafe { std::ptr::copy_nonoverlapping(cstr.as_ptr(), buf, n) };
            unsafe { (*resp_array.add(i)).resp = buf };
        }
    }

    unsafe { *resp_out = resp_array };
    PAM_SUCCESS
}

// --- Safe session wrapper ----------------------------------------------------

pub struct Session {
    pamh:   *mut pam_handle,
    /// Keep the boxed conversation alive for as long as the PAM handle.
    _conv:  Box<Box<dyn Conversation>>,
    creds_established: bool,
    session_open:      bool,
}

#[derive(Debug)]
pub enum PamError {
    Generic { code: c_int, message: String },
}

impl std::fmt::Display for PamError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PamError::Generic { code, message } =>
                write!(f, "PAM error {code}: {message}"),
        }
    }
}

impl std::error::Error for PamError {}

fn strerror(pamh: *const pam_handle, code: c_int) -> String {
    let s = unsafe { pam_strerror(pamh, code) };
    if s.is_null() { return format!("(code {code})"); }
    unsafe { CStr::from_ptr(s) }.to_string_lossy().into_owned()
}

impl Session {
    /// Start a PAM transaction. The `conv` value is moved into the
    /// session and used to answer any prompts PAM raises.
    pub fn start(service: &str, user: Option<&str>, conv: Box<dyn Conversation>) -> Result<Self, PamError> {
        let service_c = CString::new(service)
            .map_err(|_| PamError::Generic { code: -1, message: "service name has NUL".into() })?;
        let user_c = match user {
            Some(u) => Some(CString::new(u)
                .map_err(|_| PamError::Generic { code: -1, message: "user name has NUL".into() })?),
            None => None,
        };

        // Box::new the trait object once, then Box-of-Box so we have a
        // stable address to pass through PAM (the inner Box's address).
        let mut boxed: Box<Box<dyn Conversation>> = Box::new(conv);

        let conv_struct = pam_conv {
            conv:        conv_trampoline,
            appdata_ptr: &mut *boxed as *mut Box<dyn Conversation> as *mut c_void,
        };

        let mut pamh: *mut pam_handle = ptr::null_mut();
        let user_ptr = user_c.as_ref().map(|c| c.as_ptr()).unwrap_or(ptr::null());
        let rc = unsafe {
            pam_start(service_c.as_ptr(), user_ptr, &conv_struct, &mut pamh)
        };
        if rc != PAM_SUCCESS {
            return Err(PamError::Generic { code: rc, message: strerror(pamh, rc) });
        }
        Ok(Self {
            pamh,
            _conv: boxed,
            creds_established: false,
            session_open:      false,
        })
    }

    pub fn authenticate(&mut self) -> Result<(), PamError> { self.call(unsafe_pam_authenticate, "pam_authenticate") }
    pub fn acct_mgmt   (&mut self) -> Result<(), PamError> { self.call(unsafe_pam_acct_mgmt,    "pam_acct_mgmt")   }
    pub fn open_session(&mut self) -> Result<(), PamError> { self.session_open = true;  self.call(unsafe_pam_open_session,  "pam_open_session") }
    pub fn close_session(&mut self) -> Result<(), PamError> { let r = self.call(unsafe_pam_close_session, "pam_close_session"); self.session_open = false; r }
    pub fn establish_cred(&mut self) -> Result<(), PamError> {
        let rc = unsafe { pam_setcred(self.pamh, PAM_ESTABLISH_CRED) };
        if rc != PAM_SUCCESS {
            return Err(PamError::Generic { code: rc, message: strerror(self.pamh, rc) });
        }
        self.creds_established = true;
        Ok(())
    }
    pub fn delete_cred(&mut self) -> Result<(), PamError> {
        let rc = unsafe { pam_setcred(self.pamh, PAM_DELETE_CRED) };
        self.creds_established = false;
        if rc != PAM_SUCCESS {
            return Err(PamError::Generic { code: rc, message: strerror(self.pamh, rc) });
        }
        Ok(())
    }

    fn call(&mut self, f: unsafe fn(*mut pam_handle, c_int) -> c_int, name: &str) -> Result<(), PamError> {
        let rc = unsafe { f(self.pamh, 0) };
        if rc != PAM_SUCCESS {
            return Err(PamError::Generic { code: rc, message: format!("{name}: {}", strerror(self.pamh, rc)) });
        }
        Ok(())
    }

    /// Get the authenticated username PAM settled on. Returns `None` if
    /// PAM doesn't have one (rare).
    pub fn authenticated_user(&self) -> Option<String> {
        let mut p: *const c_void = ptr::null();
        let rc = unsafe { pam_get_item(self.pamh, PAM_USER, &mut p) };
        if rc != PAM_SUCCESS || p.is_null() { return None; }
        let s = unsafe { CStr::from_ptr(p as *const libc::c_char) };
        Some(s.to_string_lossy().into_owned())
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        if self.session_open {
            unsafe { pam_close_session(self.pamh, 0) };
        }
        if self.creds_established {
            unsafe { pam_setcred(self.pamh, PAM_DELETE_CRED) };
        }
        unsafe { pam_end(self.pamh, 0) };
    }
}

// Function-pointer wrappers because `unsafe fn` can't be passed via the
// `unsafe fn ptr` syntax the way we'd like.
unsafe fn unsafe_pam_authenticate (h: *mut pam_handle, f: c_int) -> c_int { pam_authenticate (h, f) }
unsafe fn unsafe_pam_acct_mgmt    (h: *mut pam_handle, f: c_int) -> c_int { pam_acct_mgmt    (h, f) }
unsafe fn unsafe_pam_open_session (h: *mut pam_handle, f: c_int) -> c_int { pam_open_session (h, f) }
unsafe fn unsafe_pam_close_session(h: *mut pam_handle, f: c_int) -> c_int { pam_close_session(h, f) }

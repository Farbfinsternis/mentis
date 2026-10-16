//! The standard Windows folder picker (IFileOpenDialog in folder mode). windows-sys has no COM
//! interface types, so the few methods needed are called through their vtable slots.

use std::ffi::c_void;
use std::path::{Path, PathBuf};
use windows_sys::Win32::Foundation::HWND;
use windows_sys::Win32::System::Com::{
    CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx, CoTaskMemFree,
};
use windows_sys::Win32::UI::Shell::{
    FOS_FORCEFILESYSTEM, FOS_PICKFOLDERS, FileOpenDialog, SHCreateItemFromParsingName,
    SIGDN_FILESYSPATH,
};
use windows_sys::core::{GUID, HRESULT, PCWSTR, PWSTR};

const IID_IFILEOPENDIALOG: GUID = GUID::from_u128(0xd57c7288_d4ad_4768_be02_9d969532d960);
const IID_ISHELLITEM: GUID = GUID::from_u128(0x43826d1e_e718_42ee_bc55_a1e261c37bfe);

type Method = usize; // vtable slot this module never calls

#[repr(C)]
struct UnknownVtbl {
    _query_interface: Method,
    _add_ref: Method,
    release: unsafe extern "system" fn(*mut c_void) -> u32,
}

/// IFileOpenDialog up to GetResult (slot 20), in declaration order.
#[repr(C)]
struct FileDialogVtbl {
    unknown: UnknownVtbl,
    show: unsafe extern "system" fn(*mut c_void, HWND) -> HRESULT,
    _set_file_types: Method,
    _set_file_type_index: Method,
    _get_file_type_index: Method,
    _advise: Method,
    _unadvise: Method,
    set_options: unsafe extern "system" fn(*mut c_void, u32) -> HRESULT,
    get_options: unsafe extern "system" fn(*mut c_void, *mut u32) -> HRESULT,
    _set_default_folder: Method,
    set_folder: unsafe extern "system" fn(*mut c_void, *mut c_void) -> HRESULT,
    _get_folder: Method,
    _get_current_selection: Method,
    _set_file_name: Method,
    _get_file_name: Method,
    set_title: unsafe extern "system" fn(*mut c_void, PCWSTR) -> HRESULT,
    _set_ok_button_label: Method,
    _set_file_name_label: Method,
    get_result: unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> HRESULT,
}

/// IShellItem up to GetDisplayName (slot 5).
#[repr(C)]
struct ShellItemVtbl {
    unknown: UnknownVtbl,
    _bind_to_handler: Method,
    _get_parent: Method,
    get_display_name: unsafe extern "system" fn(*mut c_void, i32, *mut PWSTR) -> HRESULT,
}

/// An owned COM interface pointer; released on drop.
struct Com<V> {
    this: *mut c_void,
    _vtbl: std::marker::PhantomData<V>,
}

impl<V> Com<V> {
    fn new(this: *mut c_void) -> Option<Self> {
        (!this.is_null()).then_some(Self {
            this,
            _vtbl: std::marker::PhantomData,
        })
    }

    fn vtbl(&self) -> &V {
        // SAFETY: a COM object starts with a pointer to its vtable, which begins with `V`.
        unsafe { &**(self.this as *const *const V) }
    }
}

impl<V> Drop for Com<V> {
    fn drop(&mut self) {
        // SAFETY: every vtable starts with IUnknown.
        unsafe {
            let unknown = &**(self.this as *const *const UnknownVtbl);
            (unknown.release)(self.this);
        }
    }
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain([0]).collect()
}

/// Lets the user pick a folder, starting in `start` (or its nearest existing parent).
/// `None` if the dialog was cancelled or could not be shown.
pub fn pick(owner: HWND, title: &str, start: &Path) -> Option<PathBuf> {
    unsafe {
        // Usually already initialized on this thread; a second call is harmless.
        CoInitializeEx(std::ptr::null(), COINIT_APARTMENTTHREADED as u32);
        let mut raw = std::ptr::null_mut();
        CoCreateInstance(
            &FileOpenDialog,
            std::ptr::null_mut(),
            CLSCTX_INPROC_SERVER,
            &IID_IFILEOPENDIALOG,
            &mut raw,
        );
        let dialog = Com::<FileDialogVtbl>::new(raw)?;
        let v = dialog.vtbl();
        let mut options = 0;
        (v.get_options)(dialog.this, &mut options);
        (v.set_options)(dialog.this, options | FOS_PICKFOLDERS | FOS_FORCEFILESYSTEM);
        (v.set_title)(dialog.this, wide(title).as_ptr());
        if let Some(existing) = start.ancestors().find(|p| p.is_dir()) {
            let mut raw = std::ptr::null_mut();
            SHCreateItemFromParsingName(
                wide(&existing.to_string_lossy()).as_ptr(),
                std::ptr::null_mut(),
                &IID_ISHELLITEM,
                &mut raw,
            );
            if let Some(folder) = Com::<ShellItemVtbl>::new(raw) {
                (v.set_folder)(dialog.this, folder.this);
            }
        }
        // Cancel returns HRESULT_FROM_WIN32(ERROR_CANCELLED), a failure code.
        if (v.show)(dialog.this, owner) < 0 {
            return None;
        }
        let mut raw = std::ptr::null_mut();
        (v.get_result)(dialog.this, &mut raw);
        let item = Com::<ShellItemVtbl>::new(raw)?;
        let mut name: PWSTR = std::ptr::null_mut();
        if (item.vtbl().get_display_name)(item.this, SIGDN_FILESYSPATH, &mut name) < 0
            || name.is_null()
        {
            return None;
        }
        let len = (0..).take_while(|&i| *name.add(i) != 0).count();
        let path = PathBuf::from(String::from_utf16_lossy(std::slice::from_raw_parts(
            name, len,
        )));
        CoTaskMemFree(name as *const c_void);
        Some(path)
    }
}

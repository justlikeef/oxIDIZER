use std::ffi::{CStr, CString};
use libc::{c_char, c_void};
use anyhow::{Result, Context};
use ox_webservice_api::AllocStrFn;
use std::path::PathBuf;

// Shared allocator that just delegates to system malloc via CString cloning
unsafe extern "C" fn malloc_allocator(_arena: *const c_void, s: *const c_char) -> *mut c_char {
    let c_str = CStr::from_ptr(s);
    // Duplicate the string onto the heap managed by us (libs)
    let owned = CString::new(c_str.to_bytes()).unwrap();
    owned.into_raw()
}

type RenderFn = unsafe extern "C" fn(
    arena: *const c_void,
    alloc_fn: AllocStrFn,
    form_def: *const c_char,
    props: *const c_char
) -> *mut c_char;

pub fn render_form(form_def_json: &str, props: &serde_json::Value) -> Result<String> {
    // 1. Locate libox_forms.so
    let lib_name = if cfg!(target_os = "linux") { "libox_forms.so" } else { "libox_forms.dylib" };
    
    let exe_path = std::env::current_exe().context("Failed to get current exe path")?;
    let exe_dir = exe_path.parent().context("Failed to get exe directory")?;
    
    // Try adjacent, then lib/
    let paths = vec![
        exe_dir.join(lib_name),
        exe_dir.join("lib").join(lib_name),
        exe_dir.join("../lib").join(lib_name),
        // Dev path
        exe_dir.join("../../ox_forms/target/debug").join(lib_name),
        PathBuf::from(format!("/var/repos/oxIDIZER/target/debug/{}", lib_name)),
    ];

    let lib_path = paths.iter().find(|p| p.exists())
        .ok_or_else(|| anyhow::anyhow!("Could not find {} in search paths", lib_name))?;

    unsafe {
        let lib = libloading::Library::new(lib_path)
            .context(format!("Failed to load library {:?}", lib_path))?;

        let render_func: libloading::Symbol<RenderFn> = lib.get(b"ox_forms_render")
            .context("Failed to find symbol 'ox_forms_render'")?;

        let c_def = CString::new(form_def_json)?;
        let c_props = CString::new(props.to_string())?;

        let raw_result = render_func(
            std::ptr::null(), // No arena needed for malloc_allocator
            malloc_allocator,
            c_def.as_ptr(),
            c_props.as_ptr()
        );

        if raw_result.is_null() {
            return Err(anyhow::anyhow!("ox_forms_render returned null"));
        }

        let result_str = CStr::from_ptr(raw_result).to_string_lossy().into_owned();
        
        // Free the pointer returned by malloc_allocator
        let _ = CString::from_raw(raw_result);
        
        Ok(result_str)
    }
}

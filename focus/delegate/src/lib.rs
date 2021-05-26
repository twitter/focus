use std::path::PathBuf;
use std::ffi::{OsString, CString, OsStr};

struct Attachment {
    repo_path: PathBuf,
    fifo_path: PathBuf,
    args: String,
    hash_raw_bytes: usize,
}

#[no_mangle]
pub extern "C" fn git_storage_init(
    repo_path: *const libc::c_uchar,
    repo_path_length: libc::size_t,
    fifo_path: *const libc::c_uchar,
    fifo_path_length: libc::size_t,
    args: *const libc::c_uchar,
    args_length: libc::size_t,
    hash_raw_bytes: libc::size_t,
    attachment: *mut *mut libc::c_void, // User attachment (will be allocated)
) -> libc::c_int {
    unsafe {
        use std::os::unix::ffi::OsStringExt;

        let to_path = |bytes: *const libc::c_uchar, len: usize| -> PathBuf {
            let byte_string = Vec::from_raw_parts(bytes as *mut libc::c_uchar, len, len);
            let os_string: OsString = OsStringExt::from_vec(byte_string);
            PathBuf::from(os_string)
        };

        *attachment = Box::into_raw(Box::new(Attachment{
            repo_path: to_path(repo_path, repo_path_length),
            fifo_path: to_path(fifo_path, fifo_path_length),
            args: String::from_raw_parts(args as *mut libc::c_uchar, args_length, args_length),
            hash_raw_bytes,
        })) as *mut libc::c_void;
    }

    0
}

#[no_mangle]
pub extern "C" fn git_storage_shutdown(
    attachment: *mut libc::c_void, // User attachment (will be allocated)
) -> libc::c_int {
    let attachment = unsafe { Box::<Attachment>::from_raw(attachment as *mut Attachment) };

    -1
}

#[no_mangle]
pub extern "C" fn git_storage_fetch_object(
    attachment: *mut libc::c_void,     // User attachment
    oid: *const libc::c_uchar,         // Object ID, whose length corresponds to |hash_raw_bytes|
    path: *const libc::c_char,         // The path to the file to write the data to
    offset: libc::off_t,               // The offset to write
    capacity: libc::size_t,            // Total capacity of the file
    header_offset: *mut libc::off_t,   // Location of the header in the file
    header_length: *mut libc::size_t,  // How long the header is
    content_offset: *mut libc::off_t,  // Where the content is in the file
    content_length: *mut libc::size_t, // How long the content is
    total_length: *mut libc::size_t,   // The total length of what was written
    new_capacity: *mut libc::size_t,   // The new capacity of the file
    atime: *mut libc::time_t,          // Access time
    mtime: *mut libc::time_t,          // Modified time
) -> libc::c_int {
    let attachment = unsafe { Box::<Attachment>::from_raw(attachment as *mut Attachment) };
    -1
}

#[no_mangle]
pub extern "C" fn git_storage_size_object(
    attachment: *mut libc::c_void, // User attachment
    oid: *const libc::c_uchar,     // Object ID, whose length corresponds to |hash_raw_bytes|
    size: *mut libc::size_t,       // Size of the object
    atime: *mut libc::time_t,      // Access time
    mtime: *mut libc::time_t,      // Modified time
) -> libc::c_int {
    let attachment = unsafe { Box::<Attachment>::from_raw(attachment as *mut Attachment) };
    -1
}

#[no_mangle]
pub extern "C" fn git_storage_write_object(
    attachment: *mut libc::c_void, // User attachment
    oid: *const libc::c_uchar,     // Object ID, whose length corresponds to |hash_raw_bytes|
    header: *const libc::c_uchar,  // The header
    header_length: libc::size_t,   // How long the header is
    body: *const libc::c_uchar,    // The body
    body_length: libc::size_t,     // How long the body is
    mtime: libc::time_t,           // Modified time
) -> libc::c_int {
    let attachment  = unsafe { Box::<Attachment>::from_raw(attachment as *mut Attachment) };
    -1
}

// TODO: Glue in terms of a trait
// TODO: Intermediate structs (duh)

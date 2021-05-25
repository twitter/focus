use std::ffi::CString;

pub extern "C" fn git_storage_init(
    attachment: *mut libc::c_void, // User attachment (will be allocated)
    repo_path: CString,
    fifo_path: CString,
    args: CString,
    hash_raw_bytes: libc::size_t,
) -> libc::c_int {
    -1
}


pub extern "C" fn git_storage_shutdown(
    attachment: *mut libc::c_void, // User attachment (will be allocated)
) -> libc::c_int {
    -1
}

pub extern "C" fn git_storage_fetch_object(
    attachment: *mut libc::c_void,     // User attachment
    oid: *const libc::c_uchar,          // Object ID
    path: CString,                     // The path to the file to write the data to
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
    -1
}

pub extern "C" fn git_storage_size_object(
    attachment: *mut libc::c_void, // User attachment
    oid: *const libc::c_uchar,      // Object ID
    size: *mut libc::size_t,       // Size of the object
    atime: *mut libc::time_t,      // Access time
    mtime: *mut libc::time_t,      // Modified time
) -> libc::c_int {
    -1
}

pub extern "C" fn git_storage_write_object(
    attachment: *mut libc::c_void, // User attachment
    oid: *const libc::c_uchar,      // Object ID
    header: *const libc::c_uchar,  // The header
    header_length: libc::size_t,   // How long the header is
    body: *const libc::c_uchar,    // The body
    body_length: libc::size_t,     // How long the body is
    mtime: libc::time_t,           // Modified time
) -> libc::c_int {
    -1
}

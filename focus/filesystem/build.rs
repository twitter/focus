#[cfg(target_os = "macos")]
fn add_os_specific_flags(builder: &mut cc::Build) {
    builder
        .flag("-D_FILE_OFFSET_BITS=64")
        .flag("-D_DARWIN_USE_64_BIT_INODE");
        // .ld_flag("-L/usr/local/lib")
        // .ld_flag("-losxfuse");
}

#[cfg(target_os = "linux")]
fn add_os_specific_flags(builder: &mut cc::Build) {
    builder
        .flag("-lfuse");
}

#[cfg(target_os = "windows")]
fn add_os_specific_flags(builder: &mut cc::Build) {
    todo!("unsupported platform!");
}

fn main() {
    println!("cargo:rerun-if-changed=loopback.c");

    let mut builder = cc::Build::new();
    builder.file("loopback.c");
    add_os_specific_flags(&mut builder);
    builder.compile("loopback");
}

use std::path::PathBuf;

/// Returns a textual description of the current process
pub fn get_process_description() -> String {
    let mut cmdline = String::new();
    for arg in std::env::args() {
        cmdline.push_str(arg.as_str());
        cmdline.push(' ');
    }
    if cmdline.ends_with(' ') {
        cmdline.pop();
    }
    let current_dir =
        std::env::current_dir().unwrap_or_else(|_| PathBuf::from("unknown-directory"));
    format!(
        "'{}' running in directory {:?} with PID {} started by {} on host {}",
        cmdline,
        current_dir,
        std::process::id(),
        whoami::username(),
        whoami::hostname()
    )
}

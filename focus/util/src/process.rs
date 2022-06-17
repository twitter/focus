use std::path::PathBuf;

/// Returns a textual description of the current process
pub fn get_process_description() -> String {
    let mut cmdline = format!(
        "{}",
        std::env::current_exe()
            .unwrap_or_else(|_| PathBuf::from("unknown-executable"))
            .display()
    );
    for arg in std::env::args() {
        cmdline.push(' ');
        cmdline.push_str(arg.as_str());
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

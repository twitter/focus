use rand::distributions::Alphanumeric;
use rand::{thread_rng, Rng};
use std::collections::HashMap;
use std::env;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

pub fn get_cwd() -> PathBuf {
    env::current_dir().unwrap_or_else(|_| PathBuf::from("no_cwd"))
}

pub fn seconds_since_time(time: SystemTime) -> u64 {
    match time.duration_since(UNIX_EPOCH) {
        Ok(t) => t.as_secs(),
        Err(_) => 0,
    }
}

/// ToolInsights has a field for performance messages that requires duration in seconds.
/// The docs don't specify precision here so we'll assume f64
pub fn duration_in_seconds(start_time: SystemTime, end_time: SystemTime) -> f64 {
    end_time
        .duration_since(start_time)
        .unwrap_or_else(|_| Duration::from_secs_f64(0 as f64))
        .as_secs_f64()
}

/// Take two HashMaps and merge them together, returning a new map.
/// TODO: Make this generic and memory efficient.
pub fn merge_maps(
    map1: Option<HashMap<String, String>>,
    map2: Option<HashMap<String, String>>,
) -> Option<HashMap<String, String>> {
    let mut final_map: HashMap<String, String> = HashMap::new();
    if let Some(map) = map1 {
        final_map.extend(map);
    }
    if let Some(map) = map2 {
        final_map.extend(map);
    }
    if !final_map.is_empty() {
        Some(final_map)
    } else {
        None
    }
}

pub fn tmp_filename(prefix: Option<&Path>) -> String {
    let mut tmp_filepath: PathBuf = PathBuf::new();

    match prefix {
        None => tmp_filepath.push(env::temp_dir().to_str().unwrap_or("/tmp")),
        Some(str) => tmp_filepath.push(shellexpand::tilde(str.to_str().unwrap()).to_string()),
    }

    tmp_filepath.push(
        thread_rng()
            .sample_iter(&Alphanumeric)
            .take(20)
            .map(char::from)
            .collect::<String>(),
    );

    tmp_filepath.to_str().unwrap().to_string()
}

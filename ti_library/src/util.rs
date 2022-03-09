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

/// Generates a temporary filename appended to the passed in prefix.
/// If there is no prefix passed to tmp_filename, the default prefix is the TMPDIR env variable.
/// If TMPDIR does not exist, the default is in /tmp.
///
/// Example: `/tmp/abcdefghijklmnopqrst`
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

#[cfg(test)]
mod tmp_filename_tests {
    use crate::util::tmp_filename;
    use std::path::Path;

    #[test]
    fn creates_temp_filename_in_given_prefix_directory() {
        let prefix = "/tmp_prefix_path";
        assert!(tmp_filename(Some(Path::new(prefix))).contains(prefix))
    }
}

#[cfg(test)]
mod merge_maps_tests {
    use super::*;

    #[test]
    fn with_two_maps() {
        let map1 = Some(HashMap::from([
            ("one".to_string(), "1".to_string()),
            ("two".to_string(), "2".to_string()),
        ]));
        let map2 = Some(HashMap::from([("three".to_string(), "3".to_string())]));
        assert_eq!(
            HashMap::from([
                ("one".to_string(), "1".to_string()),
                ("two".to_string(), "2".to_string()),
                ("three".to_string(), "3".to_string())
            ]),
            merge_maps(map1, map2).unwrap()
        )
    }

    #[test]
    fn with_one_map() {
        let map1 = Some(HashMap::from([("one".to_string(), "1".to_string())]));
        let map2 = None;
        assert_eq!(
            HashMap::from([("one".to_string(), "1".to_string()),]),
            merge_maps(map1.clone(), map2.clone()).unwrap()
        );

        assert_eq!(
            HashMap::from([("one".to_string(), "1".to_string()),]),
            merge_maps(map2.clone(), map1.clone()).unwrap()
        )
    }

    #[test]
    fn with_no_maps() {
        let map1 = None;
        let map2 = None;
        assert_eq!(None, merge_maps(map1.clone(), map2.clone()));
    }
}

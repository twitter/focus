use std::collections::HashMap;
use std::env;
use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

pub(crate) fn get_cwd() -> PathBuf {
    env::current_dir().unwrap_or_else(|_| PathBuf::from("no_cwd"))
}

pub(crate) fn seconds_since_time(time: SystemTime) -> u64 {
    match time.duration_since(UNIX_EPOCH) {
        Ok(t) => t.as_secs(),
        Err(_) => 0,
    }
}

/// ToolInsights has a field for performance messages that requires duration in seconds.
/// The docs don't specify precision here so we'll assume f64
pub(crate) fn duration_in_seconds(start_time: SystemTime, end_time: SystemTime) -> f64 {
    end_time
        .duration_since(start_time)
        .unwrap_or_else(|_| Duration::from_secs_f64(0 as f64))
        .as_secs_f64()
}

/// Take two HashMaps and merge them together, returning a new map.
/// TODO: Make this generic and memory efficient.
pub(crate) fn merge_maps(
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

#[cfg(test)]
mod tests {
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
            merge_maps(map2, map1).unwrap()
        )
    }

    #[test]
    fn with_no_maps() {
        let map1 = None;
        let map2 = None;
        assert_eq!(None, merge_maps(map1, map2));
    }
}

use rand::Rng;
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

pub(crate) fn get_zipkin_compatible_id() -> u64 {
    let range_lower: u64 = 0x1000_0000_0000_0000;
    let range_upper: u64 = 0x7fff_ffff_ffff_ffff;
    // In this range, when we convert the number to hex string, we will not need leading zeros
    // and it will not exceed i64 limits
    rand::thread_rng().gen_range(range_lower..range_upper)
}

/// We take a u64, turn that into hex string.
/// If the truncate flag is set then truncate the resulting hex number to 7.5 words.
/// The motivation for doing this comes from the current implementation in git.
pub(crate) fn encode_zipkin_compatible_id(id: u64, truncate: bool) -> String {
    let mut hex_string = format!("0x{:x}", id);
    if truncate {
        // The current use of truncate is strictly for when we are writing the json.
        // We do that to make sure that the id from focus match the id in git stats.
        let truncated_length = "0x".len() + 15; // "0x" + 7.5 words
        hex_string.truncate(truncated_length);
    }
    hex_string
}

/// Take a hex number as a String and parse that into a u64
pub(crate) fn decode_zipkin_compatible_id(id_as_str: impl Into<String>) -> u64 {
    let id_str = id_as_str.into();
    let hex_string = match id_str.starts_with("0x") {
        true => id_str[2..].to_string(),
        false => id_str,
    };
    u64::from_str_radix(&hex_string, 16).unwrap_or(0)
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

    #[test]
    fn test_encode_zipkin_compatible_id() {
        assert_eq!(
            "0x1deadbeefdeadbee",
            encode_zipkin_compatible_id(2155777191158930414, false)
        )
    }

    #[test]
    fn test_encode_zipkin_compatible_id_truncate() {
        assert_eq!(
            "0x1deadbeefdeadbe",
            encode_zipkin_compatible_id(2155777191158930414, true)
        )
    }

    #[test]
    fn test_decode_zipkin_compatible_id() {
        assert_eq!(
            2155777191158930414,
            decode_zipkin_compatible_id("0x1deadbeefdeadbee")
        )
    }

    #[test]
    fn test_decode_zipkin_compatible_id_no_prefix() {
        assert_eq!(
            2155777191158930414,
            decode_zipkin_compatible_id("1deadbeefdeadbee")
        )
    }
}

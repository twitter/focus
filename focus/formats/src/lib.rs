pub mod parachute {
    include!(concat!(env!("OUT_DIR"), "/parachute.rs"));
    // tonic::include_proto!("parachute");
}

pub mod storage {
    include!(concat!(env!("OUT_DIR"), "/parachute.rs"));
    // tonic::include_proto!("storage");
}

pub mod treesnap {
    include!(concat!(env!("OUT_DIR"), "/treesnap.rs"));
}

pub mod testing {
    include!(concat!(env!("OUT_DIR"), "/testing.rs"));
}

pub mod blaze_query {
    include!(concat!(env!("OUT_DIR"), "/blaze_query.rs"));
}

pub mod analysis {
    include!(concat!(env!("OUT_DIR"), "/analysis.rs"));
}

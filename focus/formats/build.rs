fn main() -> Result<(), Box<dyn std::error::Error>> {
    //     use std::io::Result;
    // fn main() -> Result<()> {
    prost_build::compile_protos(
        &[
            "proto/journal.proto",
            "proto/parachute.proto",
            "proto/storage.proto",
            "proto/testing.proto",
            "proto/treesnap.proto",
            "proto/blaze_query.proto",
            "proto/analysis.proto",
        ],
        &["proto/"],
    )?;
    Ok(())
}

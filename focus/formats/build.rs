fn main() -> Result<(), Box<dyn std::error::Error>> {
    tonic_build::compile_protos("proto/journal.proto")?;
    tonic_build::compile_protos("proto/parachute.proto")?;
    tonic_build::compile_protos("proto/storage.proto")?;
    tonic_build::compile_protos("proto/testing.proto")?;
    tonic_build::compile_protos("proto/treesnap.proto")?;

    tonic_build::compile_protos("proto/blaze_query.proto")?;
    tonic_build::compile_protos("proto/analysis.proto")?;

    Ok(())
}

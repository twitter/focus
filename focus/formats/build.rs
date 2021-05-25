fn main() -> Result<(), Box<dyn std::error::Error>> {
    tonic_build::compile_protos("proto/treesnap.proto")?;
    tonic_build::compile_protos("proto/storage.proto")?;

    Ok(())
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("cargo:rerun-if-changed=proto/runtime.proto");

    tonic_build::configure()
        .build_server(true)
        .build_client(true)
        .compile_protos(&["proto/runtime.proto"], &["proto"])?;

    Ok(())
}

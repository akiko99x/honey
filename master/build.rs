// compiles the shared proto contract into rust stubs at build time.
// master only needs the client side for now.
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let protoc = protoc_bin_vendored::protoc_bin_path()?;
    std::env::set_var("PROTOC", protoc);
    tonic_build::configure()
        .build_server(false)
        .build_client(true)
        .compile_protos(
            &[
                "../proto/honey/v1/agent.proto",
                "../proto/honey/v1/common.proto",
            ],
            &["../proto"],
        )?;
    Ok(())
}

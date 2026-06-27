//! Compile the gRPC `.proto` only when the `grpc` feature is enabled (so the
//! default build needs no `protoc`).

fn main() {
    if std::env::var_os("CARGO_FEATURE_GRPC").is_some() {
        tonic_build::configure()
            .build_server(true)
            .build_client(true)
            .compile_protos(&["proto/engram.proto"], &["proto"])
            .expect("failed to compile proto/engram.proto (is protoc installed?)");
    }
    println!("cargo:rerun-if-changed=proto/engram.proto");
}

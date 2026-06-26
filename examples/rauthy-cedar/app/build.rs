fn main() {
    connectrpc_build::Config::new()
        .files(&[
            "proto/demo/v1/api.proto",
            // Standard gRPC Health + Reflection specs, served by the same
            // Router as ApiImpl on BOTH hosts. We compile them ourselves
            // rather than depend on connectrpc-health / connectrpc-reflection,
            // whose manifests over-declare `connectrpc = { features =
            // ["server"] }` and so pull `mio` (compile_error! on wasm32).
            "proto/grpc/health/v1/health.proto",
            "proto/grpc/reflection/v1/reflection.proto",
        ])
        .includes(&["proto/"])
        .include_file("_connectrpc.rs")
        // Emit the FileDescriptorSet (full transitive import closure) so the
        // reflection service can answer file/symbol/list queries from the
        // embedded bytes — `include_bytes!` it in lib.rs.
        .emit_descriptor_set("app.fds.bin")
        .compile()
        .unwrap();
}

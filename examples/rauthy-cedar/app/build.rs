fn main() {
    connectrpc_build::Config::new()
        .files(&["proto/demo/v1/api.proto"])
        .includes(&["proto/"])
        .include_file("_connectrpc.rs")
        .compile()
        .unwrap();
}

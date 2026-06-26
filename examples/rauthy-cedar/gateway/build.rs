fn main() {
    // Compile ONLY the gateway's own front-door proto. The backend's
    // `demo.v1.Api` client type is reused verbatim from `rauthy-cedar-app`
    // (the shared app already exposes `proto::demo::v1::ApiClient`), so the
    // gateway never re-generates the demo proto — it depends on the app for it.
    connectrpc_build::Config::new()
        .files(&["proto/gateway/v1/gateway.proto"])
        .includes(&["proto/"])
        .include_file("_connectrpc.rs")
        .compile()
        .unwrap();
}

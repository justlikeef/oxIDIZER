fn main() {
    // Tell prost-build to use the vendored protoc compiler
    // so users don't need protoc installed globally.
    std::env::set_var("PROTOC", protoc_bin_vendored::protoc_bin_path().unwrap());

    prost_build::Config::new()
        .compile_protos(&["proto/task_state.proto"], &["proto/"])
        .unwrap();
}

//! `hnl` CLI internal library surface (enables `cargo test --lib`).
//!
//! The `hnl` binary lives in `src/main.rs`; this lib holds unit-test helpers
//! and smoke tests for the compile subcommand wiring (ADR-0114 Phase 14).

#[cfg(test)]
mod tests {
    use handl_plugin_ext_compile::CompileFileRequest;

    #[test]
    fn compile_file_request_parses_args() {
        let args = vec![
            "js".to_string(),
            "main.hnl".to_string(),
            "--output".to_string(),
            "main.js".to_string(),
            "--format".to_string(),
            "esm".to_string(),
        ];
        let req = CompileFileRequest::from_args(&args).unwrap();
        assert_eq!(req.target, "js");
        assert_eq!(req.path, std::path::PathBuf::from("main.hnl"));
        assert_eq!(req.output, Some(std::path::PathBuf::from("main.js")));
        assert_eq!(req.format, Some("esm".to_string()));
        assert!(!req.debug);
        assert!(!req.source_map);
    }

    #[test]
    fn compile_file_request_requires_two_positionals() {
        let args = vec!["js".to_string()];
        assert!(CompileFileRequest::from_args(&args).is_err());
    }

    #[test]
    fn compile_file_request_rejects_unknown_flags() {
        let args = vec![
            "js".to_string(),
            "main.hnl".to_string(),
            "--unknown".to_string(),
        ];
        assert!(CompileFileRequest::from_args(&args).is_err());
    }
}

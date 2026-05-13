use std::path::Path;

use reshape_cli::rpc_protocol::export_ts_bindings;

#[test]
fn exports_typescript_bindings_for_plugin_protocol() {
    let repo_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let output = repo_root.join("extensions/reshape/src/generated/reshape.ts");

    export_ts_bindings(&output).unwrap();

    let content = std::fs::read_to_string(output).unwrap();

    assert!(content.contains("export type PluginHandshake"));
    assert!(content.contains("export type PluginHandshakeAck"));
    assert!(content.contains("export type RpcOutput"));
    assert!(content.contains("export type ErrorCode"));
    assert!(content.contains("reshape.plugin.handshake_ack"));
}

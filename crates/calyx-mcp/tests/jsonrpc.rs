use calyx_mcp::{
    CALYX_MCP_JSONRPC_INVALID, JsonRpcWire, decode_jsonrpc_request, decode_jsonrpc_wire,
};

#[test]
fn valid_single_request_decodes() {
    let request =
        decode_jsonrpc_request(br#"{"jsonrpc":"2.0","method":"tools/list","id":1}"#).unwrap();

    assert_eq!(request.jsonrpc, "2.0");
    assert_eq!(request.method, "tools/list");
}

#[test]
fn valid_batch_decodes() {
    let wire = decode_jsonrpc_wire(
        br#"[{"jsonrpc":"2.0","method":"initialize","params":{}},{"jsonrpc":"2.0","method":"tools/list","id":"a"}]"#,
    )
    .unwrap();

    match wire {
        JsonRpcWire::Batch(requests) => assert_eq!(requests.len(), 2),
        JsonRpcWire::Single(_) => panic!("expected batch"),
    }
}

#[test]
fn malformed_wire_fails_closed_with_mcp_code() {
    let error = decode_jsonrpc_wire(br#"{"jsonrpc":"2.0","method":""}"#).unwrap_err();

    assert_eq!(error.code, CALYX_MCP_JSONRPC_INVALID);
    assert!(error.message.contains("method"));
}

#[test]
fn invalid_edges_fail_closed() {
    for bytes in [
        b"not-json".as_slice(),
        br#"[]"#,
        br#"{"jsonrpc":"1.0","method":"tools/list"}"#,
        br#"{"jsonrpc":"2.0","method":"rpc.internal"}"#,
        br#"{"jsonrpc":"2.0","method":"tools/list","params":5}"#,
    ] {
        let error = decode_jsonrpc_wire(bytes).unwrap_err();
        assert_eq!(error.code, CALYX_MCP_JSONRPC_INVALID);
    }
}

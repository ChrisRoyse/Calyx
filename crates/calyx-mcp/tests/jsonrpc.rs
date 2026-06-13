use calyx_mcp::{
    CALYX_MCP_JSONRPC_INVALID, JsonRpcId, JsonRpcRequest, JsonRpcWire, decode_jsonrpc_request,
    decode_jsonrpc_wire,
};

#[test]
fn request_id_round_trips_for_string_int_and_null() {
    // Integer id.
    let int_req =
        decode_jsonrpc_request(br#"{"jsonrpc":"2.0","id":99,"method":"tools/list"}"#).unwrap();
    assert_eq!(int_req.id, Some(JsonRpcId::Number(99)));
    let wire = serde_json::to_string(&int_req).unwrap();
    let back: JsonRpcRequest = serde_json::from_str(&wire).unwrap();
    assert_eq!(back, int_req);

    // String id.
    let str_req =
        decode_jsonrpc_request(br#"{"jsonrpc":"2.0","id":"abc","method":"tools/list"}"#).unwrap();
    assert_eq!(str_req.id, Some(JsonRpcId::String("abc".into())));

    // Explicit null id deserializes to None.
    let null_req =
        decode_jsonrpc_request(br#"{"jsonrpc":"2.0","id":null,"method":"tools/list"}"#).unwrap();
    assert_eq!(null_req.id, None);

    // Absent id also deserializes to None.
    let no_id = decode_jsonrpc_request(br#"{"jsonrpc":"2.0","method":"tools/list"}"#).unwrap();
    assert_eq!(no_id.id, None);
}

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

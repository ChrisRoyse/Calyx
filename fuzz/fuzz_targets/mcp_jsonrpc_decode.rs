#![no_main]

use calyx_mcp::decode_jsonrpc_wire;
use libfuzzer_sys::fuzz_target;

const MAX_INPUT: usize = 1 << 20;

fuzz_target!(|data: &[u8]| {
    let _ = decode_jsonrpc_wire(bounded(data));
});

fn bounded(data: &[u8]) -> &[u8] {
    &data[..data.len().min(MAX_INPUT)]
}

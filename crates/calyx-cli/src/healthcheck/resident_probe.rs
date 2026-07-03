//! #1154 — resident panel-parallelism probe.
//!
//! The resident enforces the never-sequential invariant inline: a chunk
//! whose runnable slots execute without pairwise span overlap fails with
//! `CALYX_EMBED_SEQUENTIAL_EXECUTION` (see `panel_commands/resident/
//! parallel.rs`). This probe therefore asserts parallelism by exercising
//! the real contract — one synthetic measure_batch through the live warm
//! panel — instead of re-deriving span analysis: a serialized deployment
//! makes the probe request itself fail with the structured cause.

use std::net::SocketAddr;

use calyx_core::{Input, Modality};

use super::{HealthCheck, fail, pass};
use crate::panel_commands::{measure_resident_batch_at, resident_ready_value_at};

const PROBE_ROWS: usize = 8;

/// Deterministic synthetic rows, long enough that warm transformer lenses
/// run past the resident's overlap floor (default 25ms) so the invariant
/// is actually evaluated rather than skipped as scheduling noise.
fn probe_inputs() -> Vec<Input> {
    (0..PROBE_ROWS)
        .map(|index| {
            let filler = (0..40)
                .map(|k| format!("probe-term-{index}-{k}"))
                .collect::<Vec<_>>()
                .join(" ");
            Input::new(
                Modality::Text,
                format!(
                    "calyx healthcheck panel parallelism probe row {index:02}: multi-lens \
                     measurement must overlap across warm slots. {filler}"
                )
                .into_bytes(),
            )
        })
        .collect()
}

pub(super) fn check_resident_parallelism(addr: SocketAddr) -> HealthCheck {
    let ready = match resident_ready_value_at(addr) {
        Ok(ready) => ready,
        Err(error) => {
            return fail(
                "calyx_resident_parallelism",
                "CALYX_HEALTH_RESIDENT_UNREACHABLE",
                format!("ready probe {addr}: {} {}", error.code(), error.message()),
            );
        }
    };
    if ready.get("ready").and_then(serde_json::Value::as_bool) != Some(true) {
        return fail(
            "calyx_resident_parallelism",
            "CALYX_HEALTH_RESIDENT_NOT_READY",
            format!("resident at {addr} answered ready={ready}"),
        );
    }
    let batch = match measure_resident_batch_at(addr, Modality::Text, &probe_inputs(), None) {
        Ok(batch) => batch,
        Err(error) => {
            // A serialized panel surfaces here as CALYX_EMBED_SEQUENTIAL_EXECUTION.
            return fail(
                "calyx_resident_parallelism",
                "CALYX_HEALTH_RESIDENT_PROBE_FAILED",
                format!(
                    "measure_batch probe {addr}: {} {}",
                    error.code(),
                    error.message()
                ),
            );
        }
    };
    let response = batch.response;
    if response.rows.len() != PROBE_ROWS {
        return fail(
            "calyx_resident_parallelism",
            "CALYX_HEALTH_RESIDENT_PROBE_ROWS",
            format!(
                "measure_batch probe {addr} returned {} rows for {PROBE_ROWS} inputs",
                response.rows.len()
            ),
        );
    }
    let min_measured = response
        .rows
        .iter()
        .map(|row| row.measured_slot_count)
        .min()
        .unwrap_or(0);
    if min_measured == 0 {
        return fail(
            "calyx_resident_parallelism",
            "CALYX_HEALTH_RESIDENT_NO_MEASURED_SLOTS",
            format!("measure_batch probe {addr}: a row measured zero slots"),
        );
    }
    pass(
        "calyx_resident_parallelism",
        format!(
            "resident {addr} measured {PROBE_ROWS} rows across {min_measured}+ slots in {}ms \
             with the sequential-execution gate enforced",
            response.elapsed_ms
        ),
    )
}

#[cfg(test)]
mod tests {
    use std::io::{BufRead, BufReader, Write};
    use std::net::TcpListener;

    use super::*;

    #[test]
    fn unreachable_resident_fails_the_check_with_a_typed_code() {
        // Bind then drop so the port is real but refuses connections.
        let addr = {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            listener.local_addr().unwrap()
        };
        let check = check_resident_parallelism(addr);
        assert_eq!(check.status, "fail");
        assert_eq!(check.code, Some("CALYX_HEALTH_RESIDENT_UNREACHABLE"));
    }

    #[test]
    fn not_ready_resident_fails_the_check() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            assert!(line.contains("\"ready\""));
            let mut stream = stream;
            stream
                .write_all(b"{\"ready\":false,\"schema\":\"calyx-panel-resident-readiness-v1\"}\n")
                .unwrap();
        });
        let check = check_resident_parallelism(addr);
        server.join().unwrap();
        assert_eq!(check.status, "fail");
        assert_eq!(check.code, Some("CALYX_HEALTH_RESIDENT_NOT_READY"));
    }

    #[test]
    fn probe_inputs_are_deterministic_and_long() {
        let first = probe_inputs();
        let second = probe_inputs();
        assert_eq!(first.len(), PROBE_ROWS);
        for (a, b) in first.iter().zip(&second) {
            assert_eq!(a.bytes, b.bytes);
            assert!(a.bytes.len() > 400, "row too short: {}", a.bytes.len());
        }
    }
}

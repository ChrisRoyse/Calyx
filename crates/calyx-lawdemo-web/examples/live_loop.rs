//! Live end-to-end reality check of the research-chat agent loop against the REAL
//! DeepSeek API. No secret is embedded: credentials come from the environment.
//!
//! Run (from a shell that has the real creds in env, e.g. synced from Infisical):
//!   DEEPSEEK_API_KEY=... LEGAL_DEEPSEEK_BASE_URL=https://api.deepseek.com \
//!   LEGAL_DEEPSEEK_MODEL=deepseek-v4-flash \
//!   cargo run -p calyx-lawdemo-web --example live_loop -- "your question here"
//!
//! It drives the ACTUAL `run_episode` with the production [`UreqTransport`] and a stub
//! tool runner that returns realistic Calyx JSON shapes (so the Calyx binary/resident
//! are not needed here — the orchestrator FSVs the real tools live on the GPU host). It
//! prints every emitted event so you can inspect the reasoning, tool mining, citation
//! validation, and final grounded answer — the source of truth for "the loop works".

use std::sync::Arc;
use std::time::Duration;

use calyx_lawdemo_web::chat::deepseek::{ChatTransport, UreqTransport};
use calyx_lawdemo_web::chat::event::{ChatEvent, CollectingSink};
use calyx_lawdemo_web::chat::tools::{ToolOutcome, ToolRunner};
use calyx_lawdemo_web::chat::{Budget, run_episode};
use serde_json::{Value, json};

/// A stub runner returning realistic Calyx shapes with stable cx_ids so citations can
/// validate. NOT mock DATA in the doctrine sense — it stands in for the resident only
/// for this transport-level reality check; the real tools are FSV'd live.
struct StubRunner;

impl ToolRunner for StubRunner {
    async fn run(&self, name: String, args: Value) -> ToolOutcome {
        // Edge-case fidelity: if the query is clearly out-of-corpus (a non-Ohio
        // jurisdiction), return the REAL typed shapes the system would emit — an
        // out-of-scope kernel refusal and an empty search — so the loop's honest
        // refusal behaviour can be observed end to end.
        let query_txt = args
            .get("query")
            .or_else(|| args.get("case_name"))
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_lowercase();
        let out_of_scope = ["delaware", "california", "federal", "texas", "new york"]
            .iter()
            .any(|j| query_txt.contains(j));
        if out_of_scope {
            let payload = match name.as_str() {
                "kernel_answer" => (
                    false,
                    json!({"code":"CALYX_KERNEL_QUERY_OUT_OF_SCOPE",
                           "message":"query jurisdiction conflicts with kernel scope",
                           "status":"refused",
                           "reason":"explicit_query_jurisdiction_conflicts_with_kernel_scope",
                           "detected_scope":"Delaware",
                           "allowed_state":"Ohio",
                           "allowed_counties":["Cuyahoga"],
                           "allowed_appellate_districts":["Eighth"],
                           "includes_statewide_scope":true}),
                ),
                "corpus_search" | "cite_check" => (true, json!([])),
                _ => (true, json!([])),
            };
            return ToolOutcome {
                ok: payload.0,
                payload: payload.1,
                latency_ms: 2,
            };
        }
        let payload = match name.as_str() {
            "corpus_search" | "cite_check" => json!([
                {"rank":1,"cx_id":"0dac1b2f00404b57cfebaf298257f361","score":0.8419,"provenance":{"ledger_seq":41022}},
                {"rank":2,"cx_id":"8f9b8ff1e2ed614e732c0e0f4e1ac62b","score":0.8102,"provenance":{"ledger_seq":41018}}
            ]),
            "case_read" | "case_segments" => {
                let cx = args
                    .get("cx_id")
                    .and_then(Value::as_str)
                    .unwrap_or("0dac1b2f00404b57cfebaf298257f361");
                json!([{
                    "cx_id": cx,
                    "metadata": {"case_name":"City of Cleveland v. Kalish","date_filed":"2015-03-12","opinion_id":"2704114","docket_number":"96368"},
                    "input_ref": {"pointer":"opinion/2704114","redacted":false},
                    "provenance": {"seq":41022,"hash":"9f2c8d41ab"},
                    "excerpt": "A lawfully initiated traffic stop may not be prolonged beyond the time reasonably required to complete its mission absent independent reasonable suspicion."
                }])
            }
            "kernel_answer" => json!({
                "status":"grounded","answer_id":"01KXJQ6Y8SSREK6D8NPKYMJB9G",
                "nearest_similarity":0.8419,"admission_threshold":0.8208,
                "anchor_kernel_node_id":"0dac1b2f00404b57cfebaf298257f361",
                "hops":[{"from_cx_id":"0dac1b2f00404b57cfebaf298257f361","to_cx_id":"8f9b8ff1e2ed614e732c0e0f4e1ac62b","ledger_ref":{"seq":41023,"hash":"a1b2c3"}}],
                "physical_readback":"verified"
            }),
            "ledger_verify" => json!({"status":"ok","break_at":null,"verified_count":77236}),
            _ => json!({"status":"ok"}),
        };
        ToolOutcome {
            ok: true,
            payload,
            latency_ms: 3,
        }
    }
}

#[tokio::main]
async fn main() {
    let key = std::env::var("DEEPSEEK_API_KEY").expect("DEEPSEEK_API_KEY must be set in env");
    let base = std::env::var("LEGAL_DEEPSEEK_BASE_URL")
        .unwrap_or_else(|_| "https://api.deepseek.com".to_owned());
    let model =
        std::env::var("LEGAL_DEEPSEEK_MODEL").unwrap_or_else(|_| "deepseek-v4-flash".to_owned());
    let question = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "When does prolonging a traffic stop require suppression of evidence in an Ohio criminal appeal?".to_owned());

    let transport: Arc<dyn ChatTransport> =
        Arc::new(UreqTransport::new(base, key, Duration::from_secs(120)));
    let runner = StubRunner;
    let sink = CollectingSink::new();
    let budget = Budget {
        max_tool_calls: 8,
        max_turns: 10,
        max_tokens: 3000,
        wall_timeout: Duration::from_secs(180),
    };

    println!("Q: {question}\nmodel: {model}\n---");
    let outcome = run_episode(transport, &runner, &model, &question, budget, &sink).await;

    for ev in sink.events() {
        match &ev {
            ChatEvent::Reasoning {
                turn,
                reasoning_tokens,
                text,
            } => {
                println!(
                    "[reasoning turn={turn} tokens={reasoning_tokens}] {}",
                    clip(text, 220)
                );
            }
            ChatEvent::ToolCall { name, verb, .. } => println!("[tool_call {name}] {verb}"),
            ChatEvent::ToolResult {
                name,
                ok,
                headline,
                latency_ms,
                cx_ids,
                ..
            } => {
                println!(
                    "[tool_result {name} ok={ok} {latency_ms}ms] {headline} cx_ids={cx_ids:?}"
                );
            }
            ChatEvent::Validation {
                cited,
                valid,
                invalid,
                verdict,
            } => {
                println!(
                    "[validation {verdict}] cited={cited:?} valid={valid:?} invalid={invalid:?}"
                );
            }
            ChatEvent::Answer { text } => {
                println!("\n===== ANSWER =====\n{text}\n==================")
            }
            ChatEvent::Citations { items } => {
                println!("[citations] {}", serde_json::to_string(items).unwrap())
            }
            ChatEvent::Footer {
                note,
                tool_calls_used,
                wall_ms,
                turns,
                ..
            } => {
                println!("[footer] {tool_calls_used} tools, {turns} turns, {wall_ms}ms — {note}");
            }
            ChatEvent::Error(env) => println!("[ERROR] {}", serde_json::to_string(env).unwrap()),
            _ => println!("[{}]", ev.name()),
        }
    }
    println!("\nOUTCOME: {outcome:?}");
}

fn clip(s: &str, n: usize) -> String {
    s.chars().take(n).collect()
}

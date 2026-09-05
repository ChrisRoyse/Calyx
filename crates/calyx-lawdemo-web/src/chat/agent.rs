//! The bounded agentic loop.
//!
//! One episode: system prompt + user question → the reasoning model plans and requests
//! Calyx tool calls → the server executes them against the REAL system → results are
//! fed back → iterate under a hard budget → a fully grounded, validated answer (or a
//! typed refusal). Every step emits an [`crate::chat::event::ChatEvent`] so the UI can
//! display the entire process. Fails CLOSED: an ungrounded answer is never rendered.

use std::sync::Arc;
use std::time::Instant;

use futures_util::future::join_all;
use serde_json::{Value, json};

use crate::chat::citations::{self, GroundingLedger};
use crate::chat::deepseek::{
    ChatRequest, ChatTransport, OutgoingMessage, ThinkingMode, TransportError,
};
use crate::chat::event::{ChatEvent, EventSink};
use crate::chat::tools::{self, ToolRunner};
use crate::error::{ApiError, ErrorCode};

/// The versioned system-prompt identity, surfaced in `/api/health`. Bump this when the
/// prompt file changes so live prompt-iteration runs are traceable.
pub const PROMPT_VERSION: &str = "2026-07-24.v12";

/// The prompt text, embedded so the running binary is self-contained.
pub const SYSTEM_PROMPT: &str = include_str!("system_prompt.md");

/// Per-episode budget (all env-tunable via [`crate::config::ChatConfig`]).
#[derive(Debug, Clone, Copy)]
pub struct Budget {
    pub max_tool_calls: u32,
    pub max_turns: u32,
    pub max_tokens: u32,
    pub wall_timeout: std::time::Duration,
}

/// Coarse terminal status of an episode (the events carry the detail).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EpisodeOutcome {
    /// A grounded answer was rendered.
    Answered,
    /// The answer was refused (ungrounded fabrication) or the model refused in-scope.
    Refused,
    /// The episode failed (transport, budget, protocol).
    Failed,
}

struct PreparedToolCall {
    id: String,
    name: String,
    args: Value,
    budget_exhausted: bool,
}

/// Run one episode to completion, emitting events into `sink`.
pub async fn run_episode<X: ToolRunner>(
    transport: Arc<dyn ChatTransport>,
    runner: &X,
    model: &str,
    question: &str,
    budget: Budget,
    sink: &dyn EventSink,
) -> EpisodeOutcome {
    let start = Instant::now();
    let registered_tools = tools::tool_specs();
    sink.emit(ChatEvent::EpisodeStart {
        question: question.to_owned(),
        model: model.to_owned(),
        prompt_version: PROMPT_VERSION,
        max_tool_calls: budget.max_tool_calls,
        wall_timeout_secs: budget.wall_timeout.as_secs(),
        max_tokens: budget.max_tokens,
    });

    let mut messages = vec![
        OutgoingMessage::system(SYSTEM_PROMPT),
        OutgoingMessage::user(question),
    ];
    let mut ledger = GroundingLedger::new();
    let mut tool_calls_used: u32 = 0;
    let mut revision_used = false;
    let mut budget_notice_sent = false;
    let mut kernel_succeeded = false;
    let mut kernel_notice_sent = false;
    let mut turns: u32 = 0;

    while turns < budget.max_turns {
        if start.elapsed() >= budget.wall_timeout {
            return fail(sink, ApiError::of(ErrorCode::ChatTimeout));
        }
        turns += 1;

        // --- call the reasoning model (blocking transport, off the async worker) ----
        let req = ChatRequest {
            model: model.to_owned(),
            messages: messages.clone(),
            tools: registered_tools
                .iter()
                .filter(|spec| {
                    let name = spec.pointer("/function/name").and_then(Value::as_str);
                    name != Some("corpus_search") || !kernel_succeeded
                })
                .cloned()
                .collect(),
            max_tokens: budget.max_tokens,
            reasoning_effort: "high",
            thinking: ThinkingMode { kind: "enabled" },
            stream: false,
        };
        let t = transport.clone();
        let resp = match tokio::task::spawn_blocking(move || t.complete(&req)).await {
            Ok(Ok(resp)) => resp,
            Ok(Err(err)) => return fail(sink, transport_error(&err)),
            Err(join) => {
                return fail(
                    sink,
                    ApiError::new(
                        ErrorCode::ChatUpstream,
                        format!("transport task join: {join}"),
                    ),
                );
            }
        };
        let Some(choice) = resp.first().cloned() else {
            return fail(
                sink,
                ApiError::new(
                    ErrorCode::ChatUpstream,
                    "model returned no choices".to_owned(),
                ),
            );
        };
        let msg = choice.message;

        // reasoning stream (full chain-of-thought for this turn)
        if let Some(reasoning) = msg.reasoning_content.as_ref()
            && !reasoning.trim().is_empty()
        {
            sink.emit(ChatEvent::Reasoning {
                turn: turns,
                text: reasoning.clone(),
                reasoning_tokens: resp.reasoning_tokens(),
            });
        }

        let content = msg.content.clone().unwrap_or_default();
        let tool_calls = msg.tool_calls.clone().unwrap_or_default();
        if !tool_calls.is_empty()
            && msg
                .reasoning_content
                .as_deref()
                .is_none_or(|reasoning| reasoning.trim().is_empty())
        {
            return fail(
                sink,
                ApiError::new(
                    ErrorCode::ChatProtocol,
                    "thinking model returned tool calls without required reasoning_content"
                        .to_owned(),
                ),
            );
        }

        // DeepSeek thinking mode requires the full reasoning content to be replayed
        // with assistant tool-call turns on every subsequent request.
        messages.push(OutgoingMessage::assistant(
            content.clone(),
            msg.reasoning_content.clone(),
            tool_calls.clone(),
        ));

        // ---- FINAL ANSWER PATH (no tool calls) -------------------------------------
        if tool_calls.is_empty() {
            if content.trim().is_empty() {
                if !revision_used {
                    revision_used = true;
                    messages.push(OutgoingMessage::user(
                        "Your last message was empty. Write your final cited answer now.",
                    ));
                    continue;
                }
                return fail(
                    sink,
                    ApiError::new(
                        ErrorCode::ChatProtocol,
                        "model produced an empty answer".to_owned(),
                    ),
                );
            }

            let verdict = citations::validate(&content, &ledger);
            let verdict_label = verdict_label(&verdict, ledger.has_any());
            sink.emit(ChatEvent::Validation {
                cited: verdict.cited.clone(),
                valid: verdict.valid.clone(),
                invalid: verdict.invalid.clone(),
                verdict: verdict_label,
            });

            // Fabricated citation → one revision, then hard refusal (never render it).
            if !verdict.is_grounded() {
                if !revision_used {
                    revision_used = true;
                    messages.push(OutgoingMessage::user(revise_fabricated(
                        &verdict.invalid,
                        ledger.citable_text(),
                    )));
                    continue;
                }
                let err = ApiError::of(ErrorCode::ChatUngrounded).with_upstream(json!({
                    "fabricated_cx_ids": verdict.invalid,
                    "note": "cited cx_ids that no tool returned; answer refused rather than rendered",
                }));
                return refuse(sink, err);
            }

            // Grounded but cited nothing while citable material exists → one nudge.
            if !verdict.has_citations() && ledger.has_any() && !revision_used {
                revision_used = true;
                messages.push(OutgoingMessage::user(nudge_cite(ledger.citable_text())));
                continue;
            }

            // Accept: emit the validated answer, its citations, and the footer.
            sink.emit(ChatEvent::Answer {
                text: content.clone(),
            });
            sink.emit(ChatEvent::Citations {
                items: citation_items(&verdict.valid, &ledger),
            });
            sink.emit(ChatEvent::Footer {
                tool_calls_used,
                max_tool_calls: budget.max_tool_calls,
                wall_ms: start.elapsed().as_millis() as u64,
                turns,
                note: footer_note(tool_calls_used, ledger.has_any()),
            });
            sink.emit(ChatEvent::Done);
            return EpisodeOutcome::Answered;
        }

        // ---- TOOL DISPATCH PATH ----------------------------------------------------
        // Any prose alongside tool calls is the model's per-step rationale.
        if !content.trim().is_empty() {
            sink.emit(ChatEvent::Rationale {
                turn: turns,
                text: content.clone(),
            });
        }

        let mut prepared = Vec::with_capacity(tool_calls.len());
        for (index, call) in tool_calls.iter().enumerate() {
            let name = call.function.name.clone();
            let args: Value = serde_json::from_str(&call.function.arguments)
                .unwrap_or_else(|_| json!({"_raw_arguments": call.function.arguments}));
            let opinion_label = human_opinion_label(&ledger, &args);

            sink.emit(ChatEvent::ToolCall {
                turn: turns,
                index,
                id: call.id.clone(),
                name: name.clone(),
                args: args.clone(),
                verb: tools::verb(&name, &args, opinion_label.as_deref()),
            });

            let budget_exhausted = tool_calls_used >= budget.max_tool_calls;
            if !budget_exhausted {
                tool_calls_used += 1;
            }
            prepared.push(PreparedToolCall {
                id: call.id.clone(),
                name,
                args,
                budget_exhausted,
            });
        }

        let concurrent = prepared
            .iter()
            .filter(|call| !call.budget_exhausted)
            .all(|call| tools::is_read_only(&call.name));
        let outcomes = if concurrent {
            let runnable = prepared
                .iter()
                .filter(|call| !call.budget_exhausted)
                .count();
            let gpu_exclusive = prepared
                .iter()
                .filter(|call| !call.budget_exhausted && tools::is_gpu_exclusive(&call.name))
                .count();
            if runnable > 1 {
                tracing::info!(
                    turn = turns,
                    tool_count = runnable,
                    gpu_exclusive_count = gpu_exclusive,
                    "executing resource-bounded read-only tool batch"
                );
            }
            join_all(prepared.iter().map(|call| async {
                if call.budget_exhausted {
                    tools::ToolOutcome {
                        ok: false,
                        payload: ApiError::of(ErrorCode::ChatBudget).body(),
                        latency_ms: 0,
                    }
                } else {
                    runner.run(call.name.clone(), call.args.clone()).await
                }
            }))
            .await
        } else {
            let mut outcomes = Vec::with_capacity(prepared.len());
            for call in &prepared {
                outcomes.push(if call.budget_exhausted {
                    tools::ToolOutcome {
                        ok: false,
                        payload: ApiError::of(ErrorCode::ChatBudget).body(),
                        latency_ms: 0,
                    }
                } else {
                    runner.run(call.name.clone(), call.args.clone()).await
                });
            }
            outcomes
        };

        let mut kernel_failure = None;
        let mut kernel_refusal = None;
        for (call, outcome) in prepared.iter().zip(outcomes) {
            let mut cx_ids = std::collections::BTreeSet::new();
            if outcome.ok && tools::returns_citable_authority(&call.name) {
                citations::collect_cx_ids(&outcome.payload, &mut cx_ids);
                ledger.ingest(&call.id, &call.name, &outcome.payload);
            }
            sink.emit(ChatEvent::ToolResult {
                id: call.id.clone(),
                name: call.name.clone(),
                ok: outcome.ok,
                headline: if call.budget_exhausted {
                    "tool budget exhausted".to_owned()
                } else {
                    tools::headline(&call.name, outcome.ok, &outcome.payload)
                },
                latency_ms: outcome.latency_ms,
                cx_ids: cx_ids.into_iter().collect(),
                payload: outcome.payload.clone(),
            });
            if call.name == "kernel_answer" {
                if !outcome.ok {
                    kernel_failure = Some(outcome.payload.clone());
                } else {
                    match outcome.payload.get("status").and_then(Value::as_str) {
                        Some("grounded") => kernel_succeeded = true,
                        Some("refused") => kernel_refusal = Some(outcome.payload.clone()),
                        status => {
                            kernel_failure = Some(json!({
                                "code": "CALYX_LAWDEMO_KERNEL_STATUS_INVALID",
                                "message": "kernel_answer returned neither grounded nor refused",
                                "observed_status": status,
                            }));
                        }
                    }
                }
            }
            messages.push(OutgoingMessage::tool(
                call.id.clone(),
                compact(&outcome.payload),
            ));
        }
        if let Some(upstream) = kernel_failure {
            return fail(
                sink,
                ApiError::new(
                    ErrorCode::ChatProtocol,
                    "primary doctrinal kernel failed; episode stopped without substituting another retriever"
                        .to_owned(),
                )
                .with_upstream(upstream),
            );
        }
        if let Some(upstream) = kernel_refusal {
            return refuse(
                sink,
                ApiError::new(
                    ErrorCode::ChatUngrounded,
                    "the primary doctrinal kernel refused this question; no substitute retriever was used"
                        .to_owned(),
                )
                .with_upstream(upstream),
            );
        }
        if kernel_succeeded && !kernel_notice_sent {
            kernel_notice_sent = true;
            messages.push(OutgoingMessage::user(
                "The grounded doctrine kernel succeeded. corpus_search is now unavailable for \
                 this episode. Read only the smallest necessary set of authorities returned by \
                 kernel_answer, then answer from their retained text.",
            ));
        }

        // Once the budget is spent, tell the model to stop mining and answer.
        if tool_calls_used >= budget.max_tool_calls && !budget_notice_sent {
            budget_notice_sent = true;
            messages.push(OutgoingMessage::user(
                "Tool budget reached. Do not call any more tools. Write your final cited \
                 answer now using only the cx_ids already returned.",
            ));
        }
    }

    // Ran out of turns without a final answer.
    fail(
        sink,
        ApiError::new(
            ErrorCode::ChatBudget,
            "reached the maximum reasoning turns without a final answer".to_owned(),
        ),
    )
}

/// Resolve a user-facing opinion identifier from authority metadata already returned
/// in this episode. Content-addressed cx_ids are deliberately never used as labels.
fn human_opinion_label(ledger: &GroundingLedger, args: &Value) -> Option<String> {
    let cx_id = args.get("cx_id").and_then(Value::as_str)?;
    let provenance = ledger.provenance_of(cx_id)?;
    provenance
        .case_name
        .clone()
        .or_else(|| {
            provenance
                .docket_number
                .as_ref()
                .map(|docket| format!("docket {docket}"))
        })
        .or_else(|| {
            provenance
                .opinion_id
                .as_ref()
                .map(|opinion_id| format!("CourtListener opinion #{opinion_id}"))
        })
}

fn transport_error(err: &TransportError) -> ApiError {
    match err {
        TransportError::Upstream { status, body } => {
            let mut b = body.clone();
            b.truncate(600);
            ApiError::of(ErrorCode::ChatUpstream).with_upstream(json!({
                "http_status": status,
                "body": b,
            }))
        }
        other => ApiError::new(ErrorCode::ChatUpstream, other.summary()),
    }
}

fn fail(sink: &dyn EventSink, err: ApiError) -> EpisodeOutcome {
    sink.emit(ChatEvent::error(&err));
    sink.emit(ChatEvent::Done);
    EpisodeOutcome::Failed
}

fn refuse(sink: &dyn EventSink, err: ApiError) -> EpisodeOutcome {
    sink.emit(ChatEvent::error(&err));
    sink.emit(ChatEvent::Done);
    EpisodeOutcome::Refused
}

fn verdict_label(verdict: &citations::CitationVerdict, had_material: bool) -> &'static str {
    if !verdict.is_grounded() {
        "fabricated_citation"
    } else if verdict.has_citations() {
        "grounded"
    } else if had_material {
        "uncited_but_material_exists"
    } else {
        "no_evidence"
    }
}

fn allowed_citation_ids(ids: &std::collections::BTreeSet<String>) -> String {
    if ids.is_empty() {
        "NONE (no successful case_text read produced citable opinion text)".to_owned()
    } else {
        ids.iter()
            .map(|id| format!("`{id}`"))
            .collect::<Vec<_>>()
            .join(", ")
    }
}

fn revise_fabricated(
    invalid: &[String],
    citable_text: &std::collections::BTreeSet<String>,
) -> String {
    format!(
        "GROUNDING ERROR: these citation ids are not backed by a successful case_text \
         read: {}. The ONLY eligible citation ids are: {}. Copy an eligible full id \
         verbatim after the `cx:` prefix. Never use ellipses, angle-bracket placeholders, \
         shortened ids, search-only ids, or invented ids. Revise the answer now; if no \
         eligible id supports a claim, remove that claim or state the corpus limitation.",
        invalid.join(", "),
        allowed_citation_ids(citable_text)
    )
}

fn nudge_cite(citable_text: &std::collections::BTreeSet<String>) -> String {
    format!(
        "Your answer made legal claims but cited no retained opinion text. The ONLY \
         eligible citation ids from successful case_text reads are: {}. Build each \
         citation marker by placing one full eligible id verbatim after the `cx:` prefix. \
         Never use ellipses, placeholders, shortened ids, search-only ids, or invented \
         ids. Cite every legal claim or state explicitly that the retained opinions do \
         not support the claim. Rewrite now.",
        allowed_citation_ids(citable_text)
    )
}

fn footer_note(tool_calls_used: u32, had_material: bool) -> String {
    if had_material {
        format!(
            "This entire episode ran on the local Calyx system. {tool_calls_used} tool \
             calls; every cited fact traces to the append-only ledger."
        )
    } else {
        format!("{tool_calls_used} tool calls; no citable corpus material was retrieved.")
    }
}

fn citation_items(valid: &[String], ledger: &GroundingLedger) -> Vec<Value> {
    valid
        .iter()
        .map(|cx| {
            let p = ledger.provenance_of(cx);
            json!({
                "cx_id": cx,
                "verified": true,
                "case_name": p.and_then(|p| p.case_name.clone()),
                "year": p.and_then(|p| p.year.clone()),
                "tool_call_id": p.and_then(|p| p.tool_call_id.clone()),
                "tool_name": p.and_then(|p| p.tool_name.clone()),
                "ledger_seq": p.and_then(|p| p.ledger_seq),
                "source": {
                    "url": p.and_then(|p| p.source_url.clone()),
                    "download_url": p.and_then(|p| p.source_download_url.clone()),
                    "court_id": p.and_then(|p| p.court_id.clone()),
                    "court_name": p.and_then(|p| p.court_name.clone()),
                    "appellate_district": p.and_then(|p| p.appellate_district.clone()),
                    "state": p.and_then(|p| p.state.clone()),
                    "date_filed": p.and_then(|p| p.date_filed.clone()),
                    "docket_number": p.and_then(|p| p.docket_number.clone()),
                    "opinion_id": p.and_then(|p| p.opinion_id.clone()),
                    "source_dataset": p.and_then(|p| p.source_dataset.clone()),
                    "precedential_status": p.and_then(|p| p.precedential_status.clone()),
                },
            })
        })
        .collect()
}

fn compact(payload: &Value) -> String {
    serde_json::to_string(payload).unwrap_or_else(|_| "{}".to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chat::deepseek::ChatResponse;
    use crate::chat::event::CollectingSink;
    use std::sync::Mutex;

    // ---- fakes -----------------------------------------------------------------

    /// A transport that replays scripted responses in order (real recorded shapes).
    struct ScriptedTransport {
        responses: Mutex<std::collections::VecDeque<ChatResponse>>,
    }
    impl ScriptedTransport {
        fn new(responses: Vec<ChatResponse>) -> Arc<Self> {
            Arc::new(Self {
                responses: Mutex::new(responses.into()),
            })
        }
    }
    impl ChatTransport for ScriptedTransport {
        fn complete(&self, _req: &ChatRequest) -> Result<ChatResponse, TransportError> {
            self.responses
                .lock()
                .unwrap()
                .pop_front()
                .ok_or_else(|| TransportError::Network("script exhausted".into()))
        }
    }

    /// A tool runner that returns scripted payloads keyed by tool name.
    struct ScriptedRunner {
        by_name: std::collections::HashMap<String, Value>,
    }
    impl ToolRunner for ScriptedRunner {
        async fn run(&self, name: String, _args: Value) -> tools::ToolOutcome {
            let payload = self
                .by_name
                .get(&name)
                .cloned()
                .unwrap_or_else(|| json!({"code": "NO_SCRIPT"}));
            let ok = payload.get("code").is_none();
            tools::ToolOutcome {
                ok,
                payload,
                latency_ms: 1,
            }
        }
    }

    fn assistant_tool_call(name: &str, args: &str) -> ChatResponse {
        resp_from(json!({
            "choices": [{
                "finish_reason": "tool_calls",
                "message": {
                    "content": "",
                    "tool_calls": [{"index":0,"id":"call_x","type":"function",
                        "function":{"name":name,"arguments":args}}]
                }
            }]
        }))
    }
    fn assistant_answer(text: &str) -> ChatResponse {
        resp_from(json!({
            "choices": [{"finish_reason":"stop","message":{"content":text}}]
        }))
    }
    fn resp_from(v: Value) -> ChatResponse {
        serde_json::from_value(v).unwrap()
    }
    fn budget() -> Budget {
        Budget {
            max_tool_calls: 12,
            max_turns: 16,
            max_tokens: 2000,
            wall_timeout: std::time::Duration::from_secs(60),
        }
    }

    #[tokio::test]
    async fn happy_path_search_read_then_grounded_answer() {
        let transport = ScriptedTransport::new(vec![
            assistant_tool_call("corpus_search", "{\"query\":\"traffic stop\"}"),
            assistant_tool_call(
                "case_read",
                "{\"cx_id\":\"8f9b8ff1e2ed614e732c0e0f4e1ac62b\"}",
            ),
            assistant_answer(
                "## Direct Answer\nThe stop may not be prolonged \
                 [cx:8f9b8ff1e2ed614e732c0e0f4e1ac62b].",
            ),
        ]);
        let mut by_name = std::collections::HashMap::new();
        by_name.insert(
            "corpus_search".to_string(),
            json!([{"rank":1,"cx_id":"8f9b8ff1e2ed614e732c0e0f4e1ac62b","score":0.84}]),
        );
        by_name.insert(
            "case_read".to_string(),
            json!({"cx_id":"8f9b8ff1e2ed614e732c0e0f4e1ac62b",
                   "metadata":{"case_name":"City of Cleveland v. Kalish","date_filed":"2015-03-12"}}),
        );
        let runner = ScriptedRunner { by_name };
        let sink = CollectingSink::new();
        let outcome = run_episode(
            transport,
            &runner,
            "deepseek-v4-flash",
            "q?",
            budget(),
            &sink,
        )
        .await;
        assert_eq!(outcome, EpisodeOutcome::Answered);
        let names = sink.names();
        assert_eq!(names.first(), Some(&"episode_start"));
        assert!(names.contains(&"tool_call"));
        assert!(names.contains(&"tool_result"));
        assert!(names.contains(&"answer"));
        assert!(names.contains(&"citations"));
        assert_eq!(names.last(), Some(&"done"));
        // The citation carries REAL provenance read from the case_read payload.
        let cite_ev = sink
            .events()
            .into_iter()
            .find(|e| e.name() == "citations")
            .unwrap();
        let items = cite_ev.data()["items"].clone();
        assert_eq!(items[0]["case_name"], "City of Cleveland v. Kalish");
        assert_eq!(items[0]["verified"], true);
    }

    #[tokio::test]
    async fn parallel_tool_calls_are_all_executed() {
        let transport = ScriptedTransport::new(vec![
            resp_from(
                json!({"choices":[{"finish_reason":"tool_calls","message":{"content":"",
                "tool_calls":[
                    {"index":0,"id":"a","type":"function","function":{"name":"corpus_search","arguments":"{\"query\":\"x\"}"}},
                    {"index":1,"id":"b","type":"function","function":{"name":"ledger_verify","arguments":"{}"}}
                ]}}]}),
            ),
            assistant_answer("## Direct Answer\nDone, no citable material needed."),
        ]);
        let mut by_name = std::collections::HashMap::new();
        by_name.insert("corpus_search".to_string(), json!([]));
        by_name.insert("ledger_verify".to_string(), json!({"status":"ok"}));
        let runner = ScriptedRunner { by_name };
        let sink = CollectingSink::new();
        run_episode(transport, &runner, "m", "q", budget(), &sink).await;
        let tool_results = sink.names().iter().filter(|n| **n == "tool_result").count();
        assert_eq!(tool_results, 2, "both parallel tool calls executed");
    }

    #[tokio::test]
    async fn fabricated_citation_is_revised_then_refused() {
        // Model cites a cx_id no tool returned, twice → CALYX_LAWDEMO_CHAT_UNGROUNDED.
        let transport = ScriptedTransport::new(vec![
            assistant_answer("## Direct Answer\nX [cx:totally_made_up_00]."),
            assistant_answer("## Direct Answer\nStill X [cx:totally_made_up_00]."),
        ]);
        let runner = ScriptedRunner {
            by_name: std::collections::HashMap::new(),
        };
        let sink = CollectingSink::new();
        let outcome = run_episode(transport, &runner, "m", "q", budget(), &sink).await;
        assert_eq!(outcome, EpisodeOutcome::Refused);
        let err = sink
            .events()
            .into_iter()
            .find(|e| e.name() == "error")
            .unwrap();
        assert_eq!(err.data()["code"], "CALYX_LAWDEMO_CHAT_UNGROUNDED");
        // The answer text was NEVER emitted.
        assert!(!sink.names().contains(&"answer"));
    }

    #[tokio::test]
    async fn one_fabrication_is_corrected_on_revision() {
        // First answer fabricates; second uses only a real (retrieved) cx_id → accepted.
        let real = "8f9b8ff1e2ed614e732c0e0f4e1ac62b";
        let transport = ScriptedTransport::new(vec![
            assistant_tool_call("case_read", &format!("{{\"cx_id\":\"{real}\"}}")),
            assistant_answer("## Direct Answer\nX [cx:made_up_9999]."),
            assistant_answer(&format!("## Direct Answer\nX [cx:{real}].")),
        ]);
        let mut by_name = std::collections::HashMap::new();
        by_name.insert(
            "case_read".to_string(),
            json!({"cx_id": real, "metadata":{"case_name":"State v. Real"}}),
        );
        let runner = ScriptedRunner { by_name };
        let sink = CollectingSink::new();
        let outcome = run_episode(transport, &runner, "m", "q", budget(), &sink).await;
        assert_eq!(outcome, EpisodeOutcome::Answered);
    }

    #[tokio::test]
    async fn budget_exhaustion_stops_tool_execution() {
        // Budget of 1 tool call: the model keeps asking; the 2nd call is refused with a
        // budget error and the model is told to answer.
        let b = Budget {
            max_tool_calls: 1,
            ..budget()
        };
        let transport = ScriptedTransport::new(vec![
            assistant_tool_call("corpus_search", "{\"query\":\"a\"}"),
            assistant_tool_call("corpus_search", "{\"query\":\"b\"}"),
            assistant_answer("## Direct Answer\nEnough."),
        ]);
        let mut by_name = std::collections::HashMap::new();
        by_name.insert("corpus_search".to_string(), json!([]));
        let runner = ScriptedRunner { by_name };
        let sink = CollectingSink::new();
        let outcome = run_episode(transport, &runner, "m", "q", b, &sink).await;
        assert_eq!(outcome, EpisodeOutcome::Answered);
        // The 2nd tool_result must be the typed budget error.
        let budget_errs = sink
            .events()
            .into_iter()
            .filter(|e| e.name() == "tool_result" && e.data()["ok"] == false)
            .count();
        assert!(
            budget_errs >= 1,
            "a budget-exhausted tool result was emitted"
        );
    }

    #[tokio::test]
    async fn hallucinated_tool_name_yields_error_result_then_adapts() {
        // Unknown tool → typed error tool result fed back; model then answers.
        let transport = ScriptedTransport::new(vec![
            assistant_tool_call("make_up_the_law", "{}"),
            assistant_answer("## Direct Answer\nThe corpus does not support this."),
        ]);
        // Real CalyxToolRunner argv validation is unit-tested elsewhere; here the
        // scripted runner returns an error payload for the unknown tool.
        let mut by_name = std::collections::HashMap::new();
        by_name.insert(
            "make_up_the_law".to_string(),
            json!({"code":"CALYX_LAWDEMO_CHAT_TOOL_UNKNOWN"}),
        );
        let runner = ScriptedRunner { by_name };
        let sink = CollectingSink::new();
        let outcome = run_episode(transport, &runner, "m", "q", budget(), &sink).await;
        assert_eq!(outcome, EpisodeOutcome::Answered);
        let tr = sink
            .events()
            .into_iter()
            .find(|e| e.name() == "tool_result")
            .unwrap();
        assert_eq!(tr.data()["ok"], false);
    }

    #[tokio::test]
    async fn empty_answer_is_nudged_then_failed() {
        let transport = ScriptedTransport::new(vec![assistant_answer(""), assistant_answer("")]);
        let runner = ScriptedRunner {
            by_name: std::collections::HashMap::new(),
        };
        let sink = CollectingSink::new();
        let outcome = run_episode(transport, &runner, "m", "q", budget(), &sink).await;
        assert_eq!(outcome, EpisodeOutcome::Failed);
        let err = sink
            .events()
            .into_iter()
            .find(|e| e.name() == "error")
            .unwrap();
        assert_eq!(err.data()["code"], "CALYX_LAWDEMO_CHAT_PROTOCOL");
    }

    #[tokio::test]
    async fn transport_failure_emits_typed_upstream_error() {
        struct Dead;
        impl ChatTransport for Dead {
            fn complete(&self, _r: &ChatRequest) -> Result<ChatResponse, TransportError> {
                Err(TransportError::Network("connection refused".into()))
            }
        }
        let runner = ScriptedRunner {
            by_name: std::collections::HashMap::new(),
        };
        let sink = CollectingSink::new();
        let outcome = run_episode(Arc::new(Dead), &runner, "m", "q", budget(), &sink).await;
        assert_eq!(outcome, EpisodeOutcome::Failed);
        let err = sink
            .events()
            .into_iter()
            .find(|e| e.name() == "error")
            .unwrap();
        assert_eq!(err.data()["code"], "CALYX_LAWDEMO_CHAT_UPSTREAM");
    }
}

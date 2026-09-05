use std::fs;

use serde_json::Value;

use crate::raw_large_corpus::{LargeCorpusPage, LargeCorpusPaginationState};

pub(crate) fn check_pagination_chains(
    pages: &[LargeCorpusPage],
    report_parse_failures: &mut Vec<String>,
) {
    for window in pages.windows(2) {
        let previous = &window[0];
        let next = &window[1];
        if previous.dataset != next.dataset {
            continue;
        }
        let Some(previous_state) = &previous.pagination_state else {
            continue;
        };
        if previous_state.mode != "keyset" {
            continue;
        }
        let Some(next_state) = &next.pagination_state else {
            report_parse_failures.push(format!(
                "{} keyset page followed by page without pagination state",
                previous.metadata_path
            ));
            continue;
        };
        if previous_state.terminal {
            report_parse_failures.push(format!(
                "{} terminal keyset page was followed by {}",
                previous.metadata_path, next.metadata_path
            ));
            continue;
        }
        if previous_state.response_next_cursor != next_state.request_after_cursor {
            report_parse_failures.push(format!(
                "{} next cursor did not match {} request cursor",
                previous.metadata_path, next.metadata_path
            ));
        }
    }
}

pub(crate) fn check_pagination_state(
    page: &LargeCorpusPage,
    report_parse_failures: &mut Vec<String>,
) {
    let Some(state) = &page.pagination_state else {
        return;
    };
    match state.mode.as_str() {
        "offset" => check_offset_page(page, state, report_parse_failures),
        "keyset" => check_keyset_page(page, state, report_parse_failures),
        other => report_parse_failures.push(format!(
            "{} unknown pagination mode {other}",
            page.metadata_path
        )),
    }
}

fn check_offset_page(
    page: &LargeCorpusPage,
    state: &LargeCorpusPaginationState,
    report_parse_failures: &mut Vec<String>,
) {
    let Some(offset) = state.requested_offset else {
        report_parse_failures.push(format!("{} offset page missing offset", page.metadata_path));
        return;
    };
    if !page.url.contains(&format!("offset={offset}")) {
        report_parse_failures.push(format!(
            "{} offset page URL missing offset={offset}",
            page.metadata_path
        ));
    }
}

fn check_keyset_page(
    page: &LargeCorpusPage,
    state: &LargeCorpusPaginationState,
    report_parse_failures: &mut Vec<String>,
) {
    let Some(items_field) = &state.items_field else {
        report_parse_failures.push(format!(
            "{} keyset page missing items_field",
            page.metadata_path
        ));
        return;
    };
    if !page.url.contains("/keyset") {
        report_parse_failures.push(format!("{} keyset URL missing /keyset", page.metadata_path));
    }
    if state.request_after_cursor.is_some() && !page.url.contains("after_cursor=") {
        report_parse_failures.push(format!(
            "{} keyset URL missing after_cursor",
            page.metadata_path
        ));
    }
    let body = match read_body_json(page) {
        Ok(body) => body,
        Err(message) => {
            report_parse_failures.push(message);
            return;
        }
    };
    let actual_count = body
        .get(items_field)
        .and_then(Value::as_array)
        .map(Vec::len);
    if actual_count != Some(page.record_count) {
        report_parse_failures.push(format!(
            "{} keyset {items_field} count mismatch expected {} actual {:?}",
            page.body_path, page.record_count, actual_count
        ));
    }
    let actual_cursor = body
        .get("next_cursor")
        .and_then(Value::as_str)
        .filter(|cursor| !cursor.is_empty())
        .map(ToString::to_string);
    if actual_cursor != state.response_next_cursor {
        report_parse_failures.push(format!(
            "{} keyset cursor mismatch expected {:?} actual {:?}",
            page.body_path, state.response_next_cursor, actual_cursor
        ));
    }
    if state.terminal != actual_cursor.is_none() {
        report_parse_failures.push(format!(
            "{} keyset terminal mismatch expected {} actual {}",
            page.body_path,
            state.terminal,
            actual_cursor.is_none()
        ));
    }
}

fn read_body_json(page: &LargeCorpusPage) -> std::result::Result<Value, String> {
    let bytes = fs::read(&page.body_path)
        .map_err(|err| format!("{} pagination body read failed: {err}", page.body_path))?;
    serde_json::from_slice(&bytes).map_err(|err| {
        format!(
            "{} pagination body JSON decode failed: {err}",
            page.body_path
        )
    })
}

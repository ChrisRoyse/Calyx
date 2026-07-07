use std::collections::{BTreeMap, BTreeSet};

use crate::raw_docs_coverage::RawDocsCoverageRow;
use crate::raw_sources::{RawEndpointSample, RawSourceCoverage};

pub(crate) fn classify_docs_rows(
    links: Vec<(String, String)>,
    samples: &[RawEndpointSample],
    coverage: &RawSourceCoverage,
) -> (Vec<RawDocsCoverageRow>, Vec<String>) {
    let evidence = sample_evidence(samples);
    let mut artifact_failures = Vec::new();
    let rows = links
        .into_iter()
        .map(|(title, url)| {
            classify_docs_row(
                title,
                url,
                &evidence,
                samples,
                coverage,
                &mut artifact_failures,
            )
        })
        .collect::<Vec<_>>();
    (rows, artifact_failures)
}

fn sample_evidence(samples: &[RawEndpointSample]) -> BTreeMap<String, Vec<&RawEndpointSample>> {
    let mut evidence: BTreeMap<String, Vec<&RawEndpointSample>> = BTreeMap::new();
    for sample in samples.iter().filter(|sample| sample.expectation_met) {
        evidence
            .entry(normalize_docs_url(&sample.docs_url))
            .or_default()
            .push(sample);
    }
    evidence
}

fn classify_docs_row(
    title: String,
    url: String,
    evidence: &BTreeMap<String, Vec<&RawEndpointSample>>,
    samples: &[RawEndpointSample],
    coverage: &RawSourceCoverage,
    artifact_failures: &mut Vec<String>,
) -> RawDocsCoverageRow {
    let normalized_url = normalize_docs_url(&url);
    let source_family = source_family_for(&normalized_url, &title);
    if normalized_url.contains("/market-data/websocket/rtds")
        && coverage
            .unsampled_sources
            .iter()
            .any(|source| source == "RTDS equity_prices stream")
    {
        return with_issues(
            row_base(
                title,
                url,
                normalized_url,
                "websocket-rtds",
                "blocked-runtime",
                "public-read-only-blocked",
                "RTDS page is partially sampled, but this inventory has no persisted live equity_prices payload bytes; keep docs-only schema blocked until a physical capture is present.",
            ),
            vec!["#198".to_string(), "#186".to_string()],
        );
    }
    if (normalized_url.contains("/market-data/websocket/sports")
        || normalized_url.contains("/api-reference/wss/sports"))
        && coverage
            .unsampled_sources
            .iter()
            .any(|source| source == "public sports WebSocket channel")
    {
        return with_issues(
            row_base(
                title,
                url,
                normalized_url,
                "websocket-sports",
                "blocked-runtime",
                "public-read-only-blocked",
                "Sports WebSocket upgraded but emitted no JSON event payload in the latest live capture; accepted as schedule-sensitive blocked-runtime under #187.",
            ),
            vec!["#187".to_string(), "#186".to_string()],
        );
    }
    if let Some(samples) = evidence.get(&normalized_url) {
        return sampled_row(title, url, normalized_url, samples, artifact_failures);
    }
    if let Some((classification, policy_status, justification)) =
        forbidden_classification(&normalized_url)
    {
        return row_base(
            title,
            url,
            normalized_url,
            source_family,
            classification,
            policy_status,
            justification,
        );
    }
    if docs_only(&normalized_url) {
        return with_issues(
            row_base(
                title,
                url,
                normalized_url,
                source_family,
                "docs-only",
                "non-data-reference",
                "Reference or conceptual docs page; it informs policy/schema but is not a standalone raw data endpoint.",
            ),
            vec!["#186".to_string()],
        );
    }
    if parent_sampled(&source_family, samples) {
        return with_issues(
            row_base(
                title,
                url,
                normalized_url,
                source_family,
                "sampled-by-parent",
                "public-read-only",
                "Source family has physical raw samples, but this exact docs row still needs endpoint-level confirmation before #186 can close.",
            ),
            vec!["#186".to_string()],
        );
    }
    with_issues(
        row_base(
            title,
            url,
            normalized_url,
            source_family,
            "not-yet-sampled",
            "public-read-only-unverified",
            "No physical raw sample is linked to this docs row yet; #186 keeps the corpus incomplete until it is sampled or reclassified.",
        ),
        vec!["#186".to_string()],
    )
}

fn sampled_row(
    title: String,
    url: String,
    normalized_url: String,
    samples: &[&RawEndpointSample],
    artifact_failures: &mut Vec<String>,
) -> RawDocsCoverageRow {
    let mut sample_names = Vec::new();
    let mut artifact_paths = Vec::new();
    let mut source_family = None;
    for sample in samples {
        source_family.get_or_insert_with(|| sample.source.clone());
        sample_names.push(sample.name.clone());
        artifact_paths.push(sample.metadata_path.clone());
        if sample.body_exists {
            artifact_paths.push(sample.body_path.clone());
        }
        if !sample.after.metadata_exists {
            artifact_failures.push(format!(
                "sample {} metadata path missing after capture: {}",
                sample.name, sample.metadata_path
            ));
        }
        if sample.body_exists && !sample.after.body_exists {
            artifact_failures.push(format!(
                "sample {} body path missing after capture: {}",
                sample.name, sample.body_path
            ));
        }
        if sample.body_sha256 != sample.after.body_sha256 {
            artifact_failures.push(format!(
                "sample {} body SHA mismatch: expected {:?} actual {:?}",
                sample.name, sample.body_sha256, sample.after.body_sha256
            ));
        }
    }
    with_samples(
        row_base(
            title,
            url,
            normalized_url,
            source_family.unwrap_or_else(|| "unknown".to_string()),
            "sampled",
            "public-read-only",
            "Exact docs row has one or more expectation-met raw samples with metadata readback evidence.",
        ),
        sample_names,
        artifact_paths,
    )
}

fn row_base(
    title: String,
    url: String,
    normalized_url: String,
    source_family: impl Into<String>,
    classification: impl Into<String>,
    policy_status: impl Into<String>,
    justification: impl Into<String>,
) -> RawDocsCoverageRow {
    RawDocsCoverageRow {
        title,
        url,
        normalized_url,
        source_family: source_family.into(),
        classification: classification.into(),
        policy_status: policy_status.into(),
        justification: justification.into(),
        sample_names: Vec::new(),
        artifact_paths: Vec::new(),
        related_issues: Vec::new(),
    }
}

fn with_samples(
    mut row: RawDocsCoverageRow,
    sample_names: Vec<String>,
    artifact_paths: Vec<String>,
) -> RawDocsCoverageRow {
    row.sample_names = sample_names;
    row.artifact_paths = artifact_paths;
    row
}

fn with_issues(mut row: RawDocsCoverageRow, related_issues: Vec<String>) -> RawDocsCoverageRow {
    row.related_issues = related_issues;
    row
}

fn normalize_docs_url(url: &str) -> String {
    let mut value = url.trim().trim_end_matches('/').to_string();
    if let Some(index) = value.find('#') {
        value.truncate(index);
    }
    if let Some(index) = value.find('?') {
        value.truncate(index);
    }
    if value.ends_with(".md") {
        value.truncate(value.len() - ".md".len());
    }
    value.trim_end_matches('/').to_string()
}

fn source_family_for(url: &str, title: &str) -> String {
    let lower_title = title.to_ascii_lowercase();
    if url.contains("websocket/rtds") {
        "websocket-rtds"
    } else if url.contains("websocket/sports") || url.contains("/wss/sports") {
        "websocket-sports"
    } else if url.contains("websocket/market-channel") || url.contains("/wss/market") {
        "websocket-market"
    } else if url.contains("websocket/user") || url.contains("/wss/user") {
        "websocket-user"
    } else if url.contains("/wss/rfq") || url.contains("/maker/") {
        "maker-rfq"
    } else if url.contains("/api-reference/core/") || url.contains("/api-reference/misc/") {
        "data-api"
    } else if url.contains("/api-reference/comments/")
        || url.contains("/api-reference/events/")
        || url.contains("/api-reference/search/")
        || url.contains("/api-reference/series/")
        || url.contains("/api-reference/tags/")
        || url.contains("/api-reference/sports/")
        || url.contains("/api-reference/profiles/")
    {
        "gamma"
    } else if url.contains("/api-reference/market-data/")
        || url.contains("/api-reference/data/")
        || url.contains("/api-reference/markets/")
        || lower_title.contains("clob")
        || lower_title.contains("order book")
    {
        "clob"
    } else if url.contains("/api-reference/bridge/") || url.contains("/trading/bridge/") {
        "bridge"
    } else if url.contains("/api-reference/relayer") {
        "relayer"
    } else if url.contains("/api-reference/trade/") || url.contains("/trading/") {
        "trade"
    } else if url.contains("/rewards/") || url.contains("/rebates/") {
        "rewards"
    } else if url.contains("/api-reference/combo-markets/") {
        "combo-markets"
    } else if url.contains("/resources/contracts") || url.contains("/resources/blockchain-data") {
        "polygon-rpc"
    } else if url.contains("/builders/") {
        "builder"
    } else {
        "docs-only"
    }
    .to_string()
}

fn forbidden_classification(url: &str) -> Option<(&'static str, &'static str, &'static str)> {
    if url.contains("/websocket/user")
        || url.contains("/wss/user")
        || url.contains("/api-reference/relayer")
        || url.contains("asyncapi-user")
        || url.contains("/api-reference/rewards/get-earnings-for-user")
        || url.contains("/api-reference/rewards/get-total-earnings-for-user")
        || url.contains("/api-reference/rewards/get-reward-percentages-for-user")
        || url.contains("/api-reference/rewards/get-user-earnings-and-markets-configuration")
    {
        return Some((
            "forbidden-auth-user",
            "forbidden-auth-user",
            "Authenticated user/relayer data is outside Poly's local-only no-site-use boundary.",
        ));
    }
    if url.contains("/api-reference/trade/")
        || url.contains("/trading/")
        || url.contains("/market-makers/")
        || url.contains("/api-reference/maker/")
        || url.contains("/wss/rfq")
        || url.contains("asyncapi-rfq")
    {
        return Some((
            "forbidden-trading",
            "forbidden-trading",
            "Trading, order, maker, RFQ, heartbeat, or signed-order surfaces are forbidden for Poly.",
        ));
    }
    if url.contains("/api-reference/bridge/") || url.contains("/trading/bridge/") {
        return Some((
            "forbidden-financial-action",
            "forbidden-financial-action",
            "Bridge deposit/withdrawal surfaces are financial action paths, not local read-only Polymarket intelligence data.",
        ));
    }
    if url.contains("/api-reference/authentication") || url.contains("/api-reference/geoblock") {
        return Some((
            "forbidden-auth-policy",
            "policy-reference-only",
            "Authentication/geographic-restriction docs inform the boundary but must not be used for authenticated collection.",
        ));
    }
    None
}

fn docs_only(url: &str) -> bool {
    url.ends_with("/index")
        || url.ends_with("/quickstart")
        || url.contains("/concepts/")
        || url.contains("/advanced/")
        || url.contains("/dev-tooling")
        || url.contains("/clients")
        || url.contains("/resources/error-codes")
        || url.contains("/resources/referral-program")
        || url.contains("/polymarket-101")
        || url.contains("/api-reference/clients-sdks")
        || url.contains("/api-reference/rate-limits")
        || url.contains("/builders/")
        || url.contains("/api-reference/introduction")
        || url.contains("/market-data/overview")
        || url.contains("/market-data/fetching-markets")
        || url.contains("/market-data/websocket/overview")
        || url.contains("/api-spec/")
        || url.contains("/developers/open-api/")
        || url.contains("openapi")
        || url.contains("asyncapi")
        || url.contains("connect-wss")
}

fn parent_sampled(source_family: &str, samples: &[RawEndpointSample]) -> bool {
    let sampled_sources = samples
        .iter()
        .filter(|sample| sample.expectation_met)
        .map(|sample| sample.source.as_str())
        .collect::<BTreeSet<_>>();
    match source_family {
        "gamma" => sampled_sources.contains("gamma"),
        "data-api" => sampled_sources.contains("data-api"),
        "clob" => sampled_sources.contains("clob") || sampled_sources.contains("clob-post"),
        "combo-markets" => sampled_sources.contains("combo-markets"),
        "rewards" => sampled_sources.contains("rewards"),
        "websocket-market" => sampled_sources.contains("websocket-market"),
        "websocket-sports" => sampled_sources.contains("websocket-sports"),
        "websocket-rtds" => sampled_sources.contains("websocket-rtds"),
        "polygon-rpc" => {
            sampled_sources.contains("polygon-rpc") || sampled_sources.contains("goldsky-subgraph")
        }
        _ => false,
    }
}

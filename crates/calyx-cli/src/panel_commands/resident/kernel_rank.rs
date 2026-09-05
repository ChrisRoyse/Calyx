use calyx_lodestar::{TypedPanelKernelIndex, typed_panel_kernel_rank_with_nearest};

use super::codec::{decode_binary, encode_binary, read_frame, write_frame};
use super::server::ResidentService;
use super::*;

pub(super) struct ResidentKernelRank {
    pub(super) manifest_sha256: String,
    pub(super) kernel_id: CxId,
    pub(super) index_sha256: String,
    pub(super) index_bytes: u64,
    pub(super) load_ms: u128,
    pub(super) index: TypedPanelKernelIndex,
}

pub(super) fn load(vault: Option<&std::path::Path>) -> CliResult<Option<ResidentKernelRank>> {
    let Some(vault) = vault else {
        return Ok(None);
    };
    if !vault.join("idx/kernel/CURRENT").is_file() {
        return Ok(None);
    }
    let started = Instant::now();
    let (loaded, index) = crate::cmd::kernel_generation::load_current_generation_with_index(vault)?;
    let resident = ResidentKernelRank {
        manifest_sha256: loaded.manifest_sha256,
        kernel_id: loaded.manifest.kernel_id,
        index_sha256: loaded.manifest.index_json.sha256,
        index_bytes: loaded.manifest.index_json.bytes,
        load_ms: started.elapsed().as_millis(),
        index,
    };
    eprintln!(
        "CALYX_PANEL_RESIDENT_RUNTIME phase=kernel_rank_pin_ok kernel_id={} manifest_sha256={} index_sha256={} index_bytes={} index_rows={} elapsed_ms={}",
        resident.kernel_id,
        resident.manifest_sha256,
        resident.index_sha256,
        resident.index_bytes,
        resident.index.rows().len(),
        resident.load_ms
    );
    Ok(Some(resident))
}

pub(super) fn serve(
    reader: &mut dyn Read,
    writer: &mut dyn Write,
    service: &ResidentService,
) -> CliResult {
    let request = match read_frame(reader)
        .and_then(|bytes| decode_binary::<ResidentKernelRankBinaryRequest>(&bytes))
    {
        Ok(request) => request,
        Err(error) => {
            return write_response(
                writer,
                ResidentKernelRankBinaryResponse::Err {
                    code: error.code.to_string(),
                    message: error.message,
                    remediation: error.remediation.to_string(),
                },
            );
        }
    };
    let Some(resident) = service.kernel_rank.as_ref() else {
        return write_response(
            writer,
            ResidentKernelRankBinaryResponse::Err {
                code: "CALYX_KERNEL_RANK_NOT_PINNED".to_string(),
                message:
                    "resident has no verified CURRENT kernel index pinned for its vault".to_string(),
                remediation:
                    "build and publish the vault kernel, then restart the resident and verify readiness.kernel_rank"
                        .to_string(),
            },
        );
    };
    if request.protocol_version != KERNEL_RANK_BINARY_PROTOCOL_VERSION {
        return write_response(
            writer,
            ResidentKernelRankBinaryResponse::Err {
                code: "CALYX_KERNEL_RANK_PROTOCOL_MISMATCH".to_string(),
                message: format!(
                    "kernel rank protocol version {}, expected {}",
                    request.protocol_version, KERNEL_RANK_BINARY_PROTOCOL_VERSION
                ),
                remediation: "restart the resident service from the same Calyx build as the CLI"
                    .to_string(),
            },
        );
    }
    if request.manifest_sha256 != resident.manifest_sha256
        || request.kernel_id != resident.kernel_id
        || request.index_sha256 != resident.index_sha256
    {
        return write_response(
            writer,
            ResidentKernelRankBinaryResponse::Err {
                code: "CALYX_KERNEL_RANK_GENERATION_MISMATCH".to_string(),
                message: format!(
                    "requested manifest={} kernel={} index={} but resident pins manifest={} kernel={} index={}",
                    request.manifest_sha256,
                    request.kernel_id,
                    request.index_sha256,
                    resident.manifest_sha256,
                    resident.kernel_id,
                    resident.index_sha256
                ),
                remediation:
                    "restart the resident against the CURRENT vault generation; never rank against a stale or different kernel"
                        .to_string(),
            },
        );
    }
    let started = Instant::now();
    let (ranked_ids, nearest) =
        match typed_panel_kernel_rank_with_nearest(&resident.index, &request.query) {
            Ok(result) => result,
            Err(error) => {
                return write_response(
                writer,
                ResidentKernelRankBinaryResponse::Err {
                    code: error.code().to_string(),
                    message: error.to_string(),
                    remediation:
                        "repair the query panel or the pinned typed kernel index contract and retry"
                            .to_string(),
                },
            );
            }
        };
    if ranked_ids.first().copied() != Some(nearest.cx_id) {
        return write_response(
            writer,
            ResidentKernelRankBinaryResponse::Err {
                code: "CALYX_KERNEL_RANK_PROTOCOL_MISMATCH".to_string(),
                message: format!(
                    "resident full rank winner {:?} differs from evidence winner {}",
                    ranked_ids.first(),
                    nearest.cx_id
                ),
                remediation:
                    "repair the typed kernel rank implementation; ranked ids and lane evidence must agree"
                        .to_string(),
            },
        );
    }
    let elapsed_ms = started.elapsed().as_millis();
    eprintln!(
        "CALYX_PANEL_RESIDENT_RUNTIME phase=kernel_rank_ok kernel_id={} index_rows={} ranked_ids={} nearest_cx={} nearest_score={} elapsed_ms={elapsed_ms}",
        resident.kernel_id,
        resident.index.rows().len(),
        ranked_ids.len(),
        nearest.cx_id,
        nearest.score
    );
    let nearest_json = match serde_json::to_vec(&nearest) {
        Ok(bytes) if bytes.len() <= MAX_KERNEL_RANK_EVIDENCE_JSON_BYTES => bytes,
        Ok(bytes) => {
            return write_response(
                writer,
                ResidentKernelRankBinaryResponse::Err {
                    code: "CALYX_KERNEL_RANK_EVIDENCE_OVERSIZE".to_string(),
                    message: format!(
                        "nearest evidence JSON is {} bytes, above the {} byte protocol bound",
                        bytes.len(),
                        MAX_KERNEL_RANK_EVIDENCE_JSON_BYTES
                    ),
                    remediation: "reduce the bounded nearest-hit evidence contract before retrying"
                        .to_string(),
                },
            );
        }
        Err(error) => {
            return write_response(
                writer,
                ResidentKernelRankBinaryResponse::Err {
                    code: "CALYX_KERNEL_RANK_EVIDENCE_ENCODE".to_string(),
                    message: format!("encode nearest evidence JSON: {error}"),
                    remediation:
                        "repair non-finite or otherwise invalid nearest-hit evidence before retrying"
                            .to_string(),
                },
            );
        }
    };
    write_response(
        writer,
        ResidentKernelRankBinaryResponse::Ok {
            protocol_version: KERNEL_RANK_BINARY_PROTOCOL_VERSION,
            process_id: std::process::id(),
            manifest_sha256: resident.manifest_sha256.clone(),
            kernel_id: resident.kernel_id,
            index_sha256: resident.index_sha256.clone(),
            index_rows: resident.index.rows().len(),
            elapsed_ms,
            nearest_json,
            ranked_ids,
        },
    )
}

fn write_response(writer: &mut dyn Write, response: ResidentKernelRankBinaryResponse) -> CliResult {
    let bytes = encode_binary(&response)?;
    write_frame(writer, &bytes)?;
    Ok(())
}

use std::path::PathBuf;

pub(crate) const DEFAULT_VAULT_ID: &str = "01ARZ3NDEKTSV4RRFFQ69G5FAV";
const DEFAULT_VAULT_SALT: &str = "calyx-ph70-media-video";

#[derive(Clone, Debug)]
pub(crate) enum VideoCommand {
    Validate(VideoValidateRequest),
    Readback(VideoReadbackRequest),
}

#[derive(Clone, Debug)]
pub(crate) struct VideoValidateRequest {
    pub(crate) metadata: PathBuf,
    pub(crate) dataset_root: Option<PathBuf>,
    pub(crate) metrics_dir: PathBuf,
    pub(crate) vault: PathBuf,
    pub(crate) vault_id: String,
    pub(crate) vault_salt: String,
}

#[derive(Clone, Debug)]
pub(crate) struct VideoReadbackRequest {
    pub(crate) vault: PathBuf,
    pub(crate) vault_id: String,
    pub(crate) vault_salt: String,
}

pub(crate) struct VideoRequest;

impl VideoRequest {
    pub(crate) fn parse(topic: &str, args: &[String]) -> Result<VideoCommand, String> {
        match topic {
            "video-validate" => parse_validate(args).map(VideoCommand::Validate),
            "video-readback" => parse_readback(args).map(VideoCommand::Readback),
            other => Err(format!("unknown media video topic: {other}")),
        }
    }
}

fn parse_validate(args: &[String]) -> Result<VideoValidateRequest, String> {
    let mut request = VideoValidateRequest {
        metadata: PathBuf::new(),
        dataset_root: None,
        metrics_dir: PathBuf::new(),
        vault: PathBuf::new(),
        vault_id: DEFAULT_VAULT_ID.to_string(),
        vault_salt: DEFAULT_VAULT_SALT.to_string(),
    };
    let mut idx = 0;
    while idx < args.len() {
        match args[idx].as_str() {
            "--metadata" => request.metadata = value(args, idx, "--metadata")?.into(),
            "--dataset-root" => {
                request.dataset_root = Some(value(args, idx, "--dataset-root")?.into())
            }
            "--metrics-dir" => request.metrics_dir = value(args, idx, "--metrics-dir")?.into(),
            "--vault" => request.vault = value(args, idx, "--vault")?.into(),
            "--vault-id" => request.vault_id = value(args, idx, "--vault-id")?.to_string(),
            "--salt" => request.vault_salt = value(args, idx, "--salt")?.to_string(),
            other => return Err(format!("unknown media video arg: {other}")),
        }
        idx += 2;
    }
    if request.metadata.as_os_str().is_empty()
        || request.metrics_dir.as_os_str().is_empty()
        || request.vault.as_os_str().is_empty()
    {
        return Err(
            "media video validation requires --metadata, --metrics-dir, and --vault".to_string(),
        );
    }
    Ok(request)
}

fn parse_readback(args: &[String]) -> Result<VideoReadbackRequest, String> {
    let mut request = VideoReadbackRequest {
        vault: PathBuf::new(),
        vault_id: DEFAULT_VAULT_ID.to_string(),
        vault_salt: DEFAULT_VAULT_SALT.to_string(),
    };
    let mut idx = 0;
    while idx < args.len() {
        match args[idx].as_str() {
            "--vault" => request.vault = value(args, idx, "--vault")?.into(),
            "--vault-id" => request.vault_id = value(args, idx, "--vault-id")?.to_string(),
            "--salt" => request.vault_salt = value(args, idx, "--salt")?.to_string(),
            other => return Err(format!("unknown media video readback arg: {other}")),
        }
        idx += 2;
    }
    if request.vault.as_os_str().is_empty() {
        return Err("media video readback requires --vault".to_string());
    }
    Ok(request)
}

fn value<'a>(args: &'a [String], idx: usize, flag: &str) -> Result<&'a str, String> {
    args.get(idx + 1)
        .map(String::as_str)
        .ok_or_else(|| format!("{flag} requires a value"))
}

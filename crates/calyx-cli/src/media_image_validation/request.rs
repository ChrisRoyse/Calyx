use std::path::PathBuf;

pub(crate) const DEFAULT_VAULT_ID: &str = "01ARZ3NDEKTSV4RRFFQ69G5FAV";
const DEFAULT_VAULT_SALT: &str = "calyx-ph70-media-image";

#[derive(Clone, Debug)]
pub(crate) struct MediaImageRequest {
    pub(crate) samples: PathBuf,
    pub(crate) metrics_dir: PathBuf,
    pub(crate) vault: PathBuf,
    pub(crate) min_image_bits: f32,
    pub(crate) min_cross_modal_bits: f32,
    pub(crate) k: usize,
    pub(crate) vault_id: String,
    pub(crate) vault_salt: String,
}

impl MediaImageRequest {
    pub(crate) fn parse(args: &[String]) -> Result<Self, String> {
        let mut request = Self {
            samples: PathBuf::new(),
            metrics_dir: PathBuf::new(),
            vault: PathBuf::new(),
            min_image_bits: 0.05,
            min_cross_modal_bits: 0.05,
            k: 3,
            vault_id: DEFAULT_VAULT_ID.to_string(),
            vault_salt: DEFAULT_VAULT_SALT.to_string(),
        };
        let mut idx = 0;
        while idx < args.len() {
            match args[idx].as_str() {
                "--samples" => {
                    request.samples = PathBuf::from(value(args, idx, "--samples")?);
                    idx += 2;
                }
                "--metrics-dir" => {
                    request.metrics_dir = PathBuf::from(value(args, idx, "--metrics-dir")?);
                    idx += 2;
                }
                "--vault" => {
                    request.vault = PathBuf::from(value(args, idx, "--vault")?);
                    idx += 2;
                }
                "--min-image-bits" => {
                    request.min_image_bits = parse_f32(args, idx, "--min-image-bits")?;
                    idx += 2;
                }
                "--min-cross-modal-bits" => {
                    request.min_cross_modal_bits = parse_f32(args, idx, "--min-cross-modal-bits")?;
                    idx += 2;
                }
                "--k" => {
                    request.k = parse_usize(args, idx, "--k")?;
                    idx += 2;
                }
                "--vault-id" => {
                    request.vault_id = value(args, idx, "--vault-id")?.to_string();
                    idx += 2;
                }
                "--salt" => {
                    request.vault_salt = value(args, idx, "--salt")?.to_string();
                    idx += 2;
                }
                other => return Err(format!("unknown media image arg: {other}")),
            }
        }
        request.validate()?;
        Ok(request)
    }

    fn validate(&self) -> Result<(), String> {
        if self.samples.as_os_str().is_empty()
            || self.metrics_dir.as_os_str().is_empty()
            || self.vault.as_os_str().is_empty()
        {
            return Err(
                "media image validation requires --samples, --metrics-dir, and --vault".to_string(),
            );
        }
        if !self.min_image_bits.is_finite() || self.min_image_bits < 0.0 {
            return Err(
                "CALYX_FSV_MEDIA_INVALID_CONFIG: --min-image-bits must be finite and non-negative"
                    .into(),
            );
        }
        if !self.min_cross_modal_bits.is_finite() || self.min_cross_modal_bits < 0.0 {
            return Err(
                "CALYX_FSV_MEDIA_INVALID_CONFIG: --min-cross-modal-bits must be finite and non-negative"
                    .into(),
            );
        }
        if self.k == 0 {
            return Err("CALYX_FSV_MEDIA_INVALID_CONFIG: --k must be positive".into());
        }
        Ok(())
    }
}

fn parse_usize(args: &[String], idx: usize, flag: &str) -> Result<usize, String> {
    value(args, idx, flag)?
        .parse::<usize>()
        .map_err(|error| format!("invalid {flag}: {error}"))
}

fn parse_f32(args: &[String], idx: usize, flag: &str) -> Result<f32, String> {
    value(args, idx, flag)?
        .parse::<f32>()
        .map_err(|error| format!("invalid {flag}: {error}"))
}

fn value<'a>(args: &'a [String], idx: usize, flag: &str) -> Result<&'a str, String> {
    args.get(idx + 1)
        .map(String::as_str)
        .ok_or_else(|| format!("{flag} requires a value"))
}

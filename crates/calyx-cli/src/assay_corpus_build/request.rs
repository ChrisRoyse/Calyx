use std::path::PathBuf;

const DEFAULT_BATCH_SIZE: usize = 16;

#[derive(Clone, Debug)]
pub(crate) struct CorpusBuildRequest {
    pub(crate) rows_jsonl: PathBuf,
    pub(crate) out_dir: PathBuf,
    pub(crate) dataset: String,
    pub(crate) target_class: usize,
    pub(crate) manifests: Vec<PathBuf>,
    pub(crate) limit_per_class: Option<usize>,
    pub(crate) batch_size: usize,
    pub(crate) cost_override_json: Option<PathBuf>,
    pub(crate) embedding_model_id: Option<String>,
    pub(crate) worker_report: Option<PathBuf>,
}

impl CorpusBuildRequest {
    pub(crate) fn parse(args: &[String]) -> Result<Self, String> {
        let mut rows_jsonl = PathBuf::new();
        let mut out_dir = PathBuf::new();
        let mut dataset = String::new();
        let mut target_class = 0_usize;
        let mut manifests = Vec::new();
        let mut limit_per_class = None;
        let mut batch_size = DEFAULT_BATCH_SIZE;
        let mut cost_override_json = None;
        let mut embedding_model_id = None;
        let mut worker_report = None;
        let mut idx = 0;
        while idx < args.len() {
            match args[idx].as_str() {
                "--rows-jsonl" => {
                    rows_jsonl = PathBuf::from(value(args, idx, "--rows-jsonl")?);
                    idx += 2;
                }
                "--out-dir" => {
                    out_dir = PathBuf::from(value(args, idx, "--out-dir")?);
                    idx += 2;
                }
                "--dataset" => {
                    dataset = value(args, idx, "--dataset")?.to_string();
                    idx += 2;
                }
                "--target-class" => {
                    target_class = parse_usize(args, idx, "--target-class")?;
                    idx += 2;
                }
                "--manifest" => {
                    manifests.push(PathBuf::from(value(args, idx, "--manifest")?));
                    idx += 2;
                }
                "--limit-per-class" => {
                    limit_per_class = Some(parse_usize(args, idx, "--limit-per-class")?);
                    idx += 2;
                }
                "--batch-size" => {
                    batch_size = parse_usize(args, idx, "--batch-size")?;
                    idx += 2;
                }
                "--cost-override-json" => {
                    cost_override_json =
                        Some(PathBuf::from(value(args, idx, "--cost-override-json")?));
                    idx += 2;
                }
                "--embedding-model-id" => {
                    embedding_model_id =
                        Some(value(args, idx, "--embedding-model-id")?.to_string());
                    idx += 2;
                }
                "--worker-report" => {
                    worker_report = Some(PathBuf::from(value(args, idx, "--worker-report")?));
                    idx += 2;
                }
                other => return Err(format!("unknown assay corpus-build arg: {other}")),
            }
        }
        let request = Self {
            rows_jsonl,
            out_dir,
            dataset,
            target_class,
            manifests,
            limit_per_class,
            batch_size,
            cost_override_json,
            embedding_model_id,
            worker_report,
        };
        request.validate()?;
        Ok(request)
    }

    fn validate(&self) -> Result<(), String> {
        if self.rows_jsonl.as_os_str().is_empty() || self.out_dir.as_os_str().is_empty() {
            return Err(
                "CALYX_FSV_ASSAY_CORPUS_BUILD_INVALID_CONFIG: --rows-jsonl and --out-dir are required"
                    .to_string(),
            );
        }
        if self.dataset.trim().is_empty() {
            return Err(
                "CALYX_FSV_ASSAY_CORPUS_BUILD_INVALID_CONFIG: --dataset must be non-empty"
                    .to_string(),
            );
        }
        if self.worker_report.is_some() && self.manifests.len() != 1 {
            return Err(
                "CALYX_FSV_ASSAY_CORPUS_BUILD_INVALID_CONFIG: worker mode requires exactly one --manifest"
                    .to_string(),
            );
        }
        if self.worker_report.is_none() && self.manifests.is_empty() {
            return Err(
                "CALYX_FSV_ASSAY_CORPUS_BUILD_INVALID_CONFIG: provide at least one --manifest entry"
                    .to_string(),
            );
        }
        if matches!(self.limit_per_class, Some(0)) {
            return Err(
                "CALYX_FSV_ASSAY_CORPUS_BUILD_INVALID_CONFIG: --limit-per-class must be > 0"
                    .to_string(),
            );
        }
        if self.batch_size == 0 {
            return Err(
                "CALYX_FSV_ASSAY_CORPUS_BUILD_INVALID_CONFIG: --batch-size must be > 0".to_string(),
            );
        }
        if let Some(id) = &self.embedding_model_id
            && id.trim().is_empty()
        {
            return Err(
                "CALYX_FSV_ASSAY_CORPUS_BUILD_INVALID_CONFIG: --embedding-model-id must be non-empty"
                    .to_string(),
            );
        }
        Ok(())
    }

    pub(crate) fn embedding_model_id(&self, lens_names: &[String]) -> String {
        self.embedding_model_id
            .clone()
            .unwrap_or_else(|| format!("panel:{}", lens_names.join("+")))
    }
}

fn parse_usize(args: &[String], idx: usize, flag: &str) -> Result<usize, String> {
    value(args, idx, flag)?.parse::<usize>().map_err(|error| {
        format!("CALYX_FSV_ASSAY_CORPUS_BUILD_INVALID_CONFIG: invalid {flag}: {error}")
    })
}

fn value<'a>(args: &'a [String], idx: usize, flag: &str) -> Result<&'a str, String> {
    args.get(idx + 1).map(String::as_str).ok_or_else(|| {
        format!("CALYX_FSV_ASSAY_CORPUS_BUILD_INVALID_CONFIG: {flag} requires a value")
    })
}

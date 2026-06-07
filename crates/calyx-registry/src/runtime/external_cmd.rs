use std::io::{Read, Write};
use std::process::{Command, Stdio};
use std::time::Duration;

use calyx_core::{CalyxError, Input, Lens, LensId, Modality, Result, SlotShape, SlotVector};
use serde::{Deserialize, Serialize};

use crate::frozen::{FrozenLensContract, LensDType, NormPolicy, sha256_digest};
use crate::lens::ensure_input_modality;

#[derive(Clone, Debug)]
pub struct ExternalCmdLens {
    id: LensId,
    cmd: String,
    args: Vec<String>,
    modality: Modality,
    dim: u32,
    timeout: Duration,
}

#[derive(Serialize)]
struct ExternalRequest<'a> {
    modality: Modality,
    inputs: Vec<&'a [u8]>,
}

#[derive(Deserialize)]
struct ExternalResponse {
    vectors: Vec<Vec<f32>>,
}

impl ExternalCmdLens {
    pub fn new(
        name: impl Into<String>,
        cmd: impl Into<String>,
        args: Vec<String>,
        modality: Modality,
        dim: u32,
    ) -> Self {
        let name = name.into();
        let cmd = cmd.into();
        let args_text = args.join("\0");
        let weights = sha256_digest(&[cmd.as_bytes(), args_text.as_bytes()]);
        let corpus = sha256_digest(&[b"external-cmd-runtime-v1"]);
        let contract = FrozenLensContract::new(
            name,
            weights,
            corpus,
            SlotShape::Dense(dim),
            modality,
            LensDType::F32,
            NormPolicy::None,
        );
        Self {
            id: contract.lens_id(),
            cmd,
            args,
            modality,
            dim,
            timeout: Duration::from_secs(30),
        }
    }

    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    pub fn command(&self) -> (&str, &[String]) {
        (&self.cmd, &self.args)
    }
}

impl Lens for ExternalCmdLens {
    fn id(&self) -> LensId {
        self.id
    }

    fn shape(&self) -> SlotShape {
        SlotShape::Dense(self.dim)
    }

    fn modality(&self) -> Modality {
        self.modality
    }

    fn measure(&self, input: &Input) -> Result<SlotVector> {
        let mut batch = self.measure_batch(std::slice::from_ref(input))?;
        batch.pop().ok_or_else(|| {
            CalyxError::lens_unreachable(format!("external lens {} returned no vector", self.id))
        })
    }

    fn measure_batch(&self, inputs: &[Input]) -> Result<Vec<SlotVector>> {
        for input in inputs {
            ensure_input_modality(self, input)?;
        }
        let request = ExternalRequest {
            modality: self.modality,
            inputs: inputs.iter().map(|input| input.bytes.as_slice()).collect(),
        };
        let request = serde_json::to_vec(&request).map_err(|err| {
            CalyxError::lens_unreachable(format!("external request encode failed: {err}"))
        })?;
        let response = run_frame(&self.cmd, &self.args, &request, self.timeout)?;
        let response: ExternalResponse = serde_json::from_slice(&response).map_err(|err| {
            CalyxError::lens_unreachable(format!("external response decode failed: {err}"))
        })?;
        if response.vectors.len() != inputs.len() {
            return Err(CalyxError::lens_dim_mismatch(format!(
                "external lens returned {} vectors for {} inputs",
                response.vectors.len(),
                inputs.len()
            )));
        }
        response
            .vectors
            .into_iter()
            .map(|data| self.slot_from_row(data))
            .collect()
    }
}

fn run_frame(cmd: &str, args: &[String], request: &[u8], _timeout: Duration) -> Result<Vec<u8>> {
    let mut child = Command::new(cmd)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|err| CalyxError::lens_unreachable(format!("spawn {cmd} failed: {err}")))?;
    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| CalyxError::lens_unreachable("external stdin pipe missing"))?;
    let len = u32::try_from(request.len())
        .map_err(|_| CalyxError::lens_dim_mismatch("external request too large"))?;
    stdin
        .write_all(&len.to_be_bytes())
        .and_then(|_| stdin.write_all(request))
        .map_err(|err| CalyxError::lens_unreachable(format!("external write failed: {err}")))?;
    drop(stdin);

    let mut stdout = child
        .stdout
        .take()
        .ok_or_else(|| CalyxError::lens_unreachable("external stdout pipe missing"))?;
    let mut header = [0_u8; 4];
    stdout.read_exact(&mut header).map_err(|err| {
        CalyxError::lens_unreachable(format!("external response header read failed: {err}"))
    })?;
    let len = u32::from_be_bytes(header) as usize;
    let mut body = vec![0_u8; len];
    stdout.read_exact(&mut body).map_err(|err| {
        CalyxError::lens_unreachable(format!("external response body read failed: {err}"))
    })?;
    let status = child
        .wait()
        .map_err(|err| CalyxError::lens_unreachable(format!("external wait failed: {err}")))?;
    if !status.success() {
        return Err(CalyxError::lens_unreachable(format!(
            "external process exited with {status}"
        )));
    }
    Ok(body)
}

impl ExternalCmdLens {
    fn slot_from_row(&self, data: Vec<f32>) -> Result<SlotVector> {
        if data.len() != self.dim as usize {
            return Err(CalyxError::lens_dim_mismatch(format!(
                "external dim {} != expected {}",
                data.len(),
                self.dim
            )));
        }
        if data.iter().any(|value| !value.is_finite()) {
            return Err(CalyxError::lens_numerical_invariant(
                "external vector contains NaN or Inf",
            ));
        }
        Ok(SlotVector::Dense {
            dim: self.dim,
            data,
        })
    }
}

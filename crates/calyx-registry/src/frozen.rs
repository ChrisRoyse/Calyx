use calyx_core::{CalyxError, Input, Lens, LensId, Modality, Result, SlotShape, SlotVector};
use sha2::{Digest, Sha256};

use crate::lens::{ensure_input_modality, ensure_vector_shape};

/// Runtime dtype declared by a frozen lens contract.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LensDType {
    /// Dense/sparse/multi f32 vectors.
    F32,
}

impl LensDType {
    const fn as_str(self) -> &'static str {
        match self {
            Self::F32 => "f32",
        }
    }
}

/// Numerical invariant policy for emitted vectors.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum NormPolicy {
    /// Values must be finite; unit length is not required.
    Finite,
    /// Values must be finite and each vector must be unit length.
    Unit { tolerance: f32 },
}

impl NormPolicy {
    /// Unit norm with the PH18 default tolerance.
    pub const fn unit() -> Self {
        Self::Unit { tolerance: 1.0e-3 }
    }

    const fn fingerprint(self) -> &'static str {
        match self {
            Self::Finite => "finite",
            Self::Unit { .. } => "unit",
        }
    }
}

/// Frozen instrument metadata used to content-address and validate a lens.
#[derive(Clone, Debug, PartialEq)]
pub struct FrozenLensContract {
    name: String,
    weights_sha256: [u8; 32],
    corpus_hash: [u8; 32],
    shape: SlotShape,
    modality: Modality,
    dtype: LensDType,
    norm: NormPolicy,
}

impl FrozenLensContract {
    /// Creates a frozen contract from already-read weight and corpus hashes.
    pub fn new(
        name: impl Into<String>,
        weights_sha256: [u8; 32],
        corpus_hash: [u8; 32],
        shape: SlotShape,
        modality: Modality,
        dtype: LensDType,
        norm: NormPolicy,
    ) -> Self {
        Self {
            name: name.into(),
            weights_sha256,
            corpus_hash,
            shape,
            modality,
            dtype,
            norm,
        }
    }

    /// Creates the PH17 algorithmic byte-feature contract.
    pub fn algorithmic_byte_features(name: impl Into<String>, modality: Modality) -> Self {
        Self::new(
            name,
            sha256_digest(&[b"algorithmic-byte-features-v1"]),
            sha256_digest(&[b"algorithmic-data-oblivious"]),
            SlotShape::Dense(16),
            modality,
            LensDType::F32,
            NormPolicy::Finite,
        )
    }

    /// Creates the PH17 aiwonder TEI HTTP contract for `:8088`.
    pub fn tei_http_8088(name: impl Into<String>, dim: u32) -> Self {
        Self::tei_http(name, "http://127.0.0.1:8088/embed", Modality::Text, dim)
    }

    /// Creates a TEI HTTP contract for an endpoint.
    pub fn tei_http(
        name: impl Into<String>,
        endpoint: impl AsRef<str>,
        modality: Modality,
        dim: u32,
    ) -> Self {
        let endpoint = endpoint.as_ref();
        Self::new(
            name,
            sha256_digest(&[endpoint.as_bytes()]),
            sha256_digest(&[b"tei-http-runtime"]),
            SlotShape::Dense(dim),
            modality,
            LensDType::F32,
            NormPolicy::unit(),
        )
    }

    /// Stable content-addressed id for this frozen instrument.
    pub fn lens_id(&self) -> LensId {
        let shape = self.output_shape_fingerprint();
        LensId::from_parts(
            &self.name,
            &self.weights_sha256,
            &self.corpus_hash,
            shape.as_bytes(),
        )
    }

    /// Returns the declared output shape.
    pub const fn shape(&self) -> SlotShape {
        self.shape
    }

    /// Returns the accepted modality.
    pub const fn modality(&self) -> Modality {
        self.modality
    }

    /// Returns the declared dtype.
    pub const fn dtype(&self) -> LensDType {
        self.dtype
    }

    /// Returns the numerical invariant policy.
    pub const fn norm_policy(&self) -> NormPolicy {
        self.norm
    }

    /// Returns the declared weights hash.
    pub const fn weights_sha256(&self) -> [u8; 32] {
        self.weights_sha256
    }

    /// Returns a copy with one byte of the weight hash changed.
    pub fn with_mutated_weight_hash(&self) -> Self {
        let mut changed = self.clone();
        changed.weights_sha256[0] ^= 0xff;
        changed
    }

    /// Verifies id, shape, and modality before a frozen lens is accepted.
    pub fn verify_registration(&self, lens: &dyn Lens) -> Result<()> {
        let expected_id = self.lens_id();
        if lens.id() != expected_id {
            return Err(CalyxError::lens_frozen_violation(format!(
                "lens id {} != frozen contract {}",
                lens.id(),
                expected_id
            )));
        }
        if lens.shape() != self.shape {
            return Err(CalyxError::lens_dim_mismatch(format!(
                "lens {} shape {:?} != frozen {:?}",
                lens.id(),
                lens.shape(),
                self.shape
            )));
        }
        if lens.modality() != self.modality {
            return Err(CalyxError::lens_dim_mismatch(format!(
                "lens {} modality {:?} != frozen {:?}",
                lens.id(),
                lens.modality(),
                self.modality
            )));
        }
        Ok(())
    }

    /// Measures a probe twice and requires byte-identical deterministic output.
    pub fn verify_determinism_probe(&self, lens: &dyn Lens, probe: &Input) -> Result<()> {
        ensure_input_modality(lens, probe)?;
        let first = lens.measure(probe)?;
        let second = lens.measure(probe)?;
        self.verify_vector(lens.id(), &first)?;
        self.verify_vector(lens.id(), &second)?;
        let first_bytes = serde_json::to_vec(&first).map_err(|err| {
            CalyxError::lens_frozen_violation(format!("serialize first probe failed: {err}"))
        })?;
        let second_bytes = serde_json::to_vec(&second).map_err(|err| {
            CalyxError::lens_frozen_violation(format!("serialize second probe failed: {err}"))
        })?;
        if first_bytes != second_bytes {
            return Err(CalyxError::lens_frozen_violation(format!(
                "lens {} changed output for deterministic probe",
                lens.id()
            )));
        }
        Ok(())
    }

    /// Verifies an emitted vector against the frozen shape and numerical policy.
    pub fn verify_vector(&self, lens_id: LensId, vector: &SlotVector) -> Result<()> {
        ensure_vector_shape(lens_id, self.shape, vector)?;
        match self.norm {
            NormPolicy::Finite => Ok(()),
            NormPolicy::Unit { tolerance } => ensure_unit_norm(lens_id, vector, tolerance),
        }
    }

    fn output_shape_fingerprint(&self) -> String {
        format!(
            "dtype={};shape={};norm={}",
            self.dtype.as_str(),
            shape_fingerprint(self.shape),
            self.norm.fingerprint()
        )
    }
}

/// Computes a length-delimited SHA-256 digest for contract fields.
pub fn sha256_digest(parts: &[&[u8]]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    for part in parts {
        hasher.update((part.len() as u64).to_be_bytes());
        hasher.update(part);
    }
    hasher.finalize().into()
}

fn shape_fingerprint(shape: SlotShape) -> String {
    match shape {
        SlotShape::Dense(dim) => format!("dense:{dim}"),
        SlotShape::Sparse(dim) => format!("sparse:{dim}"),
        SlotShape::Multi { token_dim } => format!("multi:{token_dim}"),
    }
}

fn ensure_unit_norm(lens_id: LensId, vector: &SlotVector, tolerance: f32) -> Result<()> {
    if !tolerance.is_finite() || tolerance < 0.0 {
        return Err(CalyxError::lens_numerical_invariant(
            "invalid unit-norm tolerance",
        ));
    }
    match vector {
        SlotVector::Dense { data, .. } => ensure_one_norm(lens_id, data, tolerance),
        SlotVector::Sparse { entries, .. } => {
            let sum = entries
                .iter()
                .map(|entry| f64::from(entry.val) * f64::from(entry.val))
                .sum::<f64>();
            ensure_norm_value(lens_id, sum.sqrt(), tolerance)
        }
        SlotVector::Multi { tokens, .. } => {
            for token in tokens {
                ensure_one_norm(lens_id, token, tolerance)?;
            }
            Ok(())
        }
        SlotVector::Absent { .. } => unreachable!("shape validation rejects absent vectors"),
    }
}

fn ensure_one_norm(lens_id: LensId, values: &[f32], tolerance: f32) -> Result<()> {
    let sum = values
        .iter()
        .map(|value| f64::from(*value) * f64::from(*value))
        .sum::<f64>();
    ensure_norm_value(lens_id, sum.sqrt(), tolerance)
}

fn ensure_norm_value(lens_id: LensId, norm: f64, tolerance: f32) -> Result<()> {
    if (norm - 1.0).abs() <= f64::from(tolerance) {
        return Ok(());
    }
    Err(CalyxError::lens_numerical_invariant(format!(
        "lens {lens_id} norm {norm:.6} is outside unit tolerance {tolerance}"
    )))
}

#[cfg(test)]
mod tests;

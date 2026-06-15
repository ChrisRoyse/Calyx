use calyx_core::{CalyxError, Modality, Result};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MultimodalAxis {
    Image,
    Audio,
    Protein,
    Dna,
    Molecule,
}

impl MultimodalAxis {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Image => "image",
            Self::Audio => "audio",
            Self::Protein => "protein",
            Self::Dna => "dna",
            Self::Molecule => "molecule",
        }
    }

    pub const fn modality(self) -> Modality {
        match self {
            Self::Image => Modality::Image,
            Self::Audio => Modality::Audio,
            Self::Protein => Modality::Protein,
            Self::Dna => Modality::Dna,
            Self::Molecule => Modality::Molecule,
        }
    }

    pub fn parse(raw: &str) -> Result<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "image" | "vision" => Ok(Self::Image),
            "audio" | "speech" => Ok(Self::Audio),
            "protein" | "aa" | "amino_acid" | "amino-acid" => Ok(Self::Protein),
            "dna" | "genomic" | "nucleotide" => Ok(Self::Dna),
            "molecule" | "smiles" | "chem" | "chemical" => Ok(Self::Molecule),
            other => Err(config_invalid(format!(
                "unsupported multimodal adapter axis {other}"
            ))),
        }
    }

    pub fn from_modality(modality: Modality) -> Result<Self> {
        match modality {
            Modality::Image => Ok(Self::Image),
            Modality::Audio => Ok(Self::Audio),
            Modality::Protein => Ok(Self::Protein),
            Modality::Dna => Ok(Self::Dna),
            Modality::Molecule => Ok(Self::Molecule),
            other => Err(config_invalid(format!(
                "modality {other:?} has no PH74 multimodal adapter axis"
            ))),
        }
    }
}

impl TryFrom<&str> for MultimodalAxis {
    type Error = CalyxError;

    fn try_from(value: &str) -> Result<Self> {
        Self::parse(value)
    }
}

fn config_invalid(message: impl Into<String>) -> CalyxError {
    CalyxError {
        code: "CALYX_LENS_CONFIG_INVALID",
        message: message.into(),
        remediation: "fix the multimodal adapter lens spec",
    }
}

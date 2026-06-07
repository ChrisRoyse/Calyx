use std::error::Error;
use std::fmt;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ForgeError {
    NumericalInvariant {
        op: String,
        detail: String,
        remediation: String,
    },
    DeviceUnavailable {
        device: String,
        detail: String,
        remediation: String,
    },
    ShapeMismatch {
        expected: Vec<usize>,
        got: Vec<usize>,
        remediation: String,
    },
    Unimplemented {
        op: String,
        remediation: String,
    },
}

impl ForgeError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::NumericalInvariant { .. } => "CALYX_FORGE_NUMERICAL_INVARIANT",
            Self::DeviceUnavailable { .. } => "CALYX_FORGE_DEVICE_UNAVAILABLE",
            Self::ShapeMismatch { .. } => "CALYX_FORGE_SHAPE_MISMATCH",
            Self::Unimplemented { .. } => "CALYX_FORGE_UNIMPLEMENTED",
        }
    }

    pub fn remediation(&self) -> &str {
        match self {
            Self::NumericalInvariant { remediation, .. }
            | Self::DeviceUnavailable { remediation, .. }
            | Self::ShapeMismatch { remediation, .. }
            | Self::Unimplemented { remediation, .. } => remediation,
        }
    }
}

impl fmt::Display for ForgeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let first_line = match self {
            Self::NumericalInvariant { op, detail, .. } => {
                format!("{} op={} detail={}", self.code(), op, detail)
            }
            Self::DeviceUnavailable { device, detail, .. } => {
                format!("{} device={} detail={}", self.code(), device, detail)
            }
            Self::ShapeMismatch { expected, got, .. } => {
                format!("{} expected={expected:?} got={got:?}", self.code())
            }
            Self::Unimplemented { op, .. } => format!("{} op={op}", self.code()),
        };
        if matches!(self, Self::NumericalInvariant { .. }) {
            debug_assert!(first_line.starts_with("CALYX_FORGE_NUMERICAL_INVARIANT"));
        }
        write!(f, "{first_line}\nRemediation: {}", self.remediation())
    }
}

impl Error for ForgeError {}

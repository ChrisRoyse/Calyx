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
        match self {
            Self::NumericalInvariant { op, detail, .. } => write!(
                f,
                "{} op={} detail={} remediation={}",
                self.code(),
                op,
                detail,
                self.remediation()
            ),
            Self::DeviceUnavailable { device, detail, .. } => write!(
                f,
                "{} device={} detail={} remediation={}",
                self.code(),
                device,
                detail,
                self.remediation()
            ),
            Self::ShapeMismatch { expected, got, .. } => write!(
                f,
                "{} expected={:?} got={:?} remediation={}",
                self.code(),
                expected,
                got,
                self.remediation()
            ),
            Self::Unimplemented { op, .. } => write!(
                f,
                "{} op={} remediation={}",
                self.code(),
                op,
                self.remediation()
            ),
        }
    }
}

impl Error for ForgeError {}

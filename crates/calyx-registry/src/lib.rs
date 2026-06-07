//! Registry runtimes for frozen Calyx lenses.

pub mod lens;
pub mod runtime;

pub use calyx_core::{Input, Lens};
pub use lens::{Registry, ensure_input_modality, ensure_vector_shape};
pub use runtime::algorithmic::{AlgorithmicEncoder, AlgorithmicLens};
pub use runtime::tei_http::{DEFAULT_TEI_ENDPOINT, TeiHttpLens};

pub mod check;
mod format;
mod locale;
mod localizer;
pub mod ui_check;
pub use format::{GregorianDate, LocaleFormat, MAX_DECIMAL_PRECISION};
pub use locale::{LanguagePreference, Resolution, ResolutionReason, resolve};
pub use localizer::{Diagnostic, DiagnosticKind, Localizer};
include!(concat!(env!("OUT_DIR"), "/messages.rs"));

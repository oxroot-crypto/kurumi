//! Internal validation helpers shared by the indicator implementations.

use kurumi_core::KurumiError;

/// Rejects `NaN` and infinite observations at an indicator boundary.
///
/// A non-finite value would poison every subsequent update of a recursive
/// indicator, so the check happens before any state is mutated.
#[inline]
pub(crate) fn ensure_finite(value: f64, parameter: &'static str) -> Result<(), KurumiError> {
    if value.is_finite() {
        Ok(())
    } else {
        Err(KurumiError::InvalidParameter {
            parameter,
            reason: "must be finite".to_owned(),
        })
    }
}

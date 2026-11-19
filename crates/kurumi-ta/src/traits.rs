//! The common streaming-indicator interface.

use kurumi_core::KurumiError;

/// A technical indicator that can be updated one observation at a time.
///
/// Implementors keep all of their state internally: a ring buffer for the
/// lookback window, running sums, and recursively smoothed values. A call to
/// [`StreamingIndicator::next`] must perform `O(1)` work, must not scan the
/// window, and must not allocate or reallocate heap storage.
///
/// # Warm-up
///
/// Until the indicator has received enough observations to fill its lookback
/// window it returns `Ok(None)`. The first complete value is returned by the
/// call that completes the warm-up. This is the standard warm-up convention for
/// trading libraries: consumers do not have to special-case a partial window.
///
/// # Input and output types
///
/// Price indicators consume `f64`; candle indicators consume [`OhlcvBar`] and
/// volume-based indicators could consume other value types. Outputs are either
/// `f64` or a small `Copy` struct with named fields.
///
/// [`OhlcvBar`]: crate::OhlcvBar
///
/// # Examples
///
/// ```
/// use kurumi_ta::{Sma, StreamingIndicator};
///
/// # fn main() -> Result<(), kurumi_ta::KurumiError> {
/// let mut sma = Sma::new(3)?;
/// let inputs = [1.0, 2.0, 3.0, 4.0];
/// let outputs = sma.compute_batch(&inputs)?;
/// assert_eq!(outputs, vec![None, None, Some(2.0), Some(3.0)]);
/// # Ok(())
/// # }
/// ```
pub trait StreamingIndicator {
    /// The observation type consumed by one update.
    ///
    /// Inputs are small value types and must be `Copy`, which lets
    /// [`StreamingIndicator::compute_batch`] feed a borrowed slice without
    /// cloning.
    type Input: Copy;

    /// The value produced once the indicator is warmed up.
    type Output;

    /// Consumes one observation and returns the current indicator value.
    ///
    /// Returns `Ok(None)` while the warm-up window is still incomplete.
    ///
    /// # Errors
    ///
    /// Returns [`KurumiError::InvalidParameter`] when the observation is
    /// `NaN`, infinite, or otherwise outside the indicator's domain. The
    /// indicator state is unchanged in that case.
    fn next(&mut self, input: Self::Input) -> Result<Option<Self::Output>, KurumiError>;

    /// Returns the indicator to its initial, un-warmed state.
    ///
    /// The internal storage keeps its reserved capacity, so resuming after a
    /// reset does not allocate.
    fn reset(&mut self);

    /// Feeds every observation in `slice` through [`StreamingIndicator::next`]
    /// and collects the per-observation results.
    ///
    /// This is the vectorized batch form of the indicator. It continues from
    /// the indicator's current state; call [`StreamingIndicator::reset`] first
    /// when a fresh batch run is required. Because it runs the same state
    /// machine as the streaming form, a fresh batch run is exactly equivalent
    /// to a tick-by-tick run.
    ///
    /// # Errors
    ///
    /// Returns the first error produced by [`StreamingIndicator::next`]. The
    /// outputs computed before the failing observation are discarded.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_ta::{Roc, StreamingIndicator};
    ///
    /// # fn main() -> Result<(), kurumi_ta::KurumiError> {
    /// let mut roc = Roc::new(1)?;
    /// let values = roc.compute_batch(&[100.0, 110.0, 99.0])?;
    /// assert_eq!(values, vec![None, Some(10.0), Some(-10.0)]);
    /// # Ok(())
    /// # }
    /// ```
    fn compute_batch(
        &mut self,
        slice: &[Self::Input],
    ) -> Result<Vec<Option<Self::Output>>, KurumiError> {
        let mut outputs = Vec::with_capacity(slice.len());
        for &input in slice {
            outputs.push(self.next(input)?);
        }
        Ok(outputs)
    }
}

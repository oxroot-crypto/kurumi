//! Fixed-capacity ring buffers and their O(1) sliding-window aggregates.
//!
//! [`RingBuffer`] is the storage primitive: a heap-backed, fixed-capacity
//! first-in-first-out queue whose storage is allocated once in
//! [`RingBuffer::new`]. [`RollingWindow`] keeps a running compensated sum and
//! two monotonic deques on top of it, so the window mean, minimum, and maximum
//! are all available in `O(1)` time. [`RollingVariance`] adds a sliding Welford
//! accumulator for the population variance.
//!
//! None of these types allocate on [`RingBuffer::push`],
//! [`RollingWindow::push`], or [`RollingVariance::push`]: the backing storage
//! and the monotonic deques are reserved to their final capacity at
//! construction time.

use std::collections::VecDeque;

use kurumi_core::KurumiError;

use crate::util::ensure_finite;

/// A fixed-capacity FIFO ring buffer.
///
/// The buffer stores at most `capacity` values. Once full, each push overwrites
/// the oldest value and returns it, which is exactly the eviction signal an
/// indicator needs to undo the leaving observation. Pushing is `O(1)`, reading
/// by logical index is `O(1)`, and iteration visits the live values from oldest
/// to newest.
///
/// # Allocation
///
/// All storage is reserved by [`RingBuffer::new`]. A full buffer never grows or
/// reallocates when a value is pushed, so a streaming hot path performs no heap
/// work after construction.
///
/// # Examples
///
/// ```
/// use kurumi_ta::RingBuffer;
///
/// # fn main() -> Result<(), kurumi_ta::KurumiError> {
/// let mut window = RingBuffer::new(3)?;
/// assert_eq!(window.push(1.0), None);
/// assert_eq!(window.push(2.0), None);
/// assert_eq!(window.push(3.0), None);
/// assert_eq!(window.push(4.0), Some(1.0));
/// assert_eq!(window.get(0), Some(&2.0));
/// assert_eq!(window.get(2), Some(&4.0));
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RingBuffer<T> {
    values: VecDeque<T>,
    capacity: usize,
}

impl<T> RingBuffer<T> {
    /// Creates an empty buffer with a fixed, positive capacity.
    ///
    /// # Errors
    ///
    /// Returns [`KurumiError::InvalidParameter`] when `capacity` is zero.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_ta::RingBuffer;
    ///
    /// # fn main() -> Result<(), kurumi_ta::KurumiError> {
    /// assert!(RingBuffer::<f64>::new(0).is_err());
    /// assert_eq!(RingBuffer::<f64>::new(4)?.capacity(), 4);
    /// # Ok(())
    /// # }
    /// ```
    pub fn new(capacity: usize) -> Result<Self, KurumiError> {
        if capacity == 0 {
            return Err(KurumiError::InvalidParameter {
                parameter: "capacity",
                reason: "must be strictly positive".to_owned(),
            });
        }
        Ok(Self {
            values: VecDeque::with_capacity(capacity),
            capacity,
        })
    }

    /// Pushes `value`, returning the evicted oldest value when the buffer is
    /// already full.
    ///
    /// This method is `O(1)` and never allocates.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_ta::RingBuffer;
    ///
    /// # fn main() -> Result<(), kurumi_ta::KurumiError> {
    /// let mut buffer = RingBuffer::new(2)?;
    /// assert_eq!(buffer.push("a"), None);
    /// assert_eq!(buffer.push("b"), None);
    /// assert_eq!(buffer.push("c"), Some("a"));
    /// assert_eq!(buffer.len(), 2);
    /// # Ok(())
    /// # }
    /// ```
    #[inline]
    pub fn push(&mut self, value: T) -> Option<T> {
        let evicted = if self.values.len() == self.capacity {
            self.values.pop_front()
        } else {
            None
        };
        self.values.push_back(value);
        evicted
    }

    /// Returns the live value at logical index `index`, where `0` is the oldest
    /// value and `len - 1` is the newest.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_ta::RingBuffer;
    ///
    /// # fn main() -> Result<(), kurumi_ta::KurumiError> {
    /// let mut buffer = RingBuffer::new(3)?;
    /// buffer.push(1);
    /// buffer.push(2);
    /// assert_eq!(buffer.get(0), Some(&1));
    /// assert_eq!(buffer.get(2), None);
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    #[inline]
    pub fn get(&self, index: usize) -> Option<&T> {
        self.values.get(index)
    }

    /// Returns the oldest live value.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_ta::RingBuffer;
    ///
    /// # fn main() -> Result<(), kurumi_ta::KurumiError> {
    /// let mut buffer = RingBuffer::new(2)?;
    /// buffer.push(7);
    /// assert_eq!(buffer.front(), Some(&7));
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    #[inline]
    pub fn front(&self) -> Option<&T> {
        self.values.front()
    }

    /// Returns the newest live value.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_ta::RingBuffer;
    ///
    /// # fn main() -> Result<(), kurumi_ta::KurumiError> {
    /// let mut buffer = RingBuffer::new(2)?;
    /// buffer.push(7);
    /// buffer.push(9);
    /// assert_eq!(buffer.back(), Some(&9));
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    #[inline]
    pub fn back(&self) -> Option<&T> {
        self.values.back()
    }

    /// Iterates over the live values from oldest to newest.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_ta::RingBuffer;
    ///
    /// # fn main() -> Result<(), kurumi_ta::KurumiError> {
    /// let mut buffer = RingBuffer::new(3)?;
    /// buffer.push(1);
    /// buffer.push(2);
    /// buffer.push(3);
    /// buffer.push(4);
    /// let live: Vec<_> = buffer.iter().copied().collect();
    /// assert_eq!(live, vec![2, 3, 4]);
    /// # Ok(())
    /// # }
    /// ```
    pub fn iter(&self) -> impl Iterator<Item = &T> + '_ {
        self.values.iter()
    }

    /// Returns the number of live values.
    #[must_use]
    #[inline]
    pub fn len(&self) -> usize {
        self.values.len()
    }

    /// Returns the fixed capacity.
    #[must_use]
    #[inline]
    pub const fn capacity(&self) -> usize {
        self.capacity
    }

    /// Returns `true` when no value is stored.
    #[must_use]
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }

    /// Returns `true` when the buffer is at capacity and the next push evicts.
    #[must_use]
    #[inline]
    pub fn is_full(&self) -> bool {
        self.values.len() == self.capacity
    }

    /// Removes every value while keeping the reserved capacity.
    pub fn clear(&mut self) {
        self.values.clear();
    }
}

/// A fixed-capacity `f64` window with `O(1)` mean, minimum, and maximum.
///
/// The running sum uses Neumaier compensated summation so adding and removing
/// values does not accumulate the cancellation error of a naive running sum.
/// The minimum and maximum are maintained by two monotonic double-ended queues
/// of `(position, value)`, each of which stores at most `capacity` entries.
///
/// # Complexity
///
/// [`RollingWindow::push`], [`RollingWindow::mean`], [`RollingWindow::min`],
/// and [`RollingWindow::max`] are all `O(1)` time. Pushing fills the window and
/// never allocates: both deques are reserved to `capacity` at construction.
///
/// # Examples
///
/// ```
/// use kurumi_ta::RollingWindow;
///
/// # fn main() -> Result<(), kurumi_ta::KurumiError> {
/// let mut window = RollingWindow::new(3)?;
/// window.push(1.0)?;
/// window.push(4.0)?;
/// window.push(2.0)?;
/// assert_eq!(window.mean(), Some(7.0 / 3.0));
/// assert_eq!(window.min(), Some(1.0));
/// assert_eq!(window.max(), Some(4.0));
///
/// window.push(9.0)?;
/// assert_eq!(window.mean(), Some(5.0));
/// assert_eq!(window.min(), Some(2.0));
/// assert_eq!(window.max(), Some(9.0));
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone)]
pub struct RollingWindow {
    buffer: RingBuffer<f64>,
    sum: f64,
    compensation: f64,
    minima: VecDeque<(u128, f64)>,
    maxima: VecDeque<(u128, f64)>,
    position: u128,
}

impl RollingWindow {
    /// Creates an empty window with a fixed, positive capacity.
    ///
    /// # Errors
    ///
    /// Returns [`KurumiError::InvalidParameter`] when `capacity` is zero.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_ta::RollingWindow;
    ///
    /// # fn main() -> Result<(), kurumi_ta::KurumiError> {
    /// assert!(RollingWindow::new(0).is_err());
    /// assert_eq!(RollingWindow::new(5)?.capacity(), 5);
    /// # Ok(())
    /// # }
    /// ```
    pub fn new(capacity: usize) -> Result<Self, KurumiError> {
        let buffer = RingBuffer::new(capacity)?;
        Ok(Self {
            buffer,
            sum: 0.0,
            compensation: 0.0,
            minima: VecDeque::with_capacity(capacity),
            maxima: VecDeque::with_capacity(capacity),
            position: 0,
        })
    }

    /// Pushes one finite observation, evicting the oldest when full.
    ///
    /// # Errors
    ///
    /// Returns [`KurumiError::InvalidParameter`] for non-finite observations.
    /// Returns [`KurumiError::NumericalOverflow`] if the running sum leaves the
    /// finite range; the window is cleared before that error is returned.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_ta::RollingWindow;
    ///
    /// # fn main() -> Result<(), kurumi_ta::KurumiError> {
    /// let mut window = RollingWindow::new(2)?;
    /// window.push(3.0)?;
    /// assert!(window.push(f64::NAN).is_err());
    /// assert_eq!(window.len(), 1);
    /// # Ok(())
    /// # }
    /// ```
    #[inline]
    pub fn push(&mut self, value: f64) -> Result<(), KurumiError> {
        ensure_finite(value, "value")?;

        let capacity = self.buffer.capacity();
        let index = self.position;
        self.position = self.position.wrapping_add(1);

        if let Some(evicted) = self.buffer.push(value) {
            self.add_to_sum(-evicted);
        }
        self.add_to_sum(value);

        // `oldest` is the logical position of the first value still in the
        // window after this push.
        let oldest = index.saturating_sub((capacity - 1) as u128);
        while let Some(&(position, _)) = self.minima.front() {
            if position < oldest {
                self.minima.pop_front();
            } else {
                break;
            }
        }
        while let Some(&(position, _)) = self.maxima.front() {
            if position < oldest {
                self.maxima.pop_front();
            } else {
                break;
            }
        }
        while let Some(&(_, candidate)) = self.minima.back() {
            if candidate >= value {
                self.minima.pop_back();
            } else {
                break;
            }
        }
        while let Some(&(_, candidate)) = self.maxima.back() {
            if candidate <= value {
                self.maxima.pop_back();
            } else {
                break;
            }
        }
        self.minima.push_back((index, value));
        self.maxima.push_back((index, value));

        if !self.sum.is_finite() || !self.compensation.is_finite() {
            self.clear();
            return Err(KurumiError::NumericalOverflow {
                operation: "RollingWindow::push",
            });
        }
        Ok(())
    }

    /// Returns the arithmetic mean of the live values, or `None` when empty.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_ta::RollingWindow;
    ///
    /// # fn main() -> Result<(), kurumi_ta::KurumiError> {
    /// let mut window = RollingWindow::new(2)?;
    /// assert_eq!(window.mean(), None);
    /// window.push(1.0)?;
    /// window.push(2.0)?;
    /// assert_eq!(window.mean(), Some(1.5));
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    #[inline]
    pub fn mean(&self) -> Option<f64> {
        if self.buffer.is_empty() {
            return None;
        }
        Some((self.sum + self.compensation) / self.buffer.len() as f64)
    }

    /// Returns the smallest live value, or `None` when empty.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_ta::RollingWindow;
    ///
    /// # fn main() -> Result<(), kurumi_ta::KurumiError> {
    /// let mut window = RollingWindow::new(3)?;
    /// window.push(2.0)?;
    /// window.push(-1.0)?;
    /// assert_eq!(window.min(), Some(-1.0));
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    #[inline]
    pub fn min(&self) -> Option<f64> {
        self.minima.front().map(|&(_, value)| value)
    }

    /// Returns the largest live value, or `None` when empty.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_ta::RollingWindow;
    ///
    /// # fn main() -> Result<(), kurumi_ta::KurumiError> {
    /// let mut window = RollingWindow::new(3)?;
    /// window.push(2.0)?;
    /// window.push(-1.0)?;
    /// assert_eq!(window.max(), Some(2.0));
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    #[inline]
    pub fn max(&self) -> Option<f64> {
        self.maxima.front().map(|&(_, value)| value)
    }

    /// Returns the number of live values.
    #[must_use]
    #[inline]
    pub fn len(&self) -> usize {
        self.buffer.len()
    }

    /// Returns the fixed capacity.
    #[must_use]
    #[inline]
    pub const fn capacity(&self) -> usize {
        self.buffer.capacity()
    }

    /// Returns `true` when no value is stored.
    #[must_use]
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.buffer.is_empty()
    }

    /// Returns `true` when the window is at capacity.
    #[must_use]
    #[inline]
    pub fn is_full(&self) -> bool {
        self.buffer.is_full()
    }

    /// Clears the values, aggregates, and position counter while keeping the
    /// reserved capacity.
    pub fn clear(&mut self) {
        self.buffer.clear();
        self.minima.clear();
        self.maxima.clear();
        self.sum = 0.0;
        self.compensation = 0.0;
        self.position = 0;
    }

    /// Adds `value` to the compensated running sum (Neumaier summation).
    #[inline]
    fn add_to_sum(&mut self, value: f64) {
        let next = self.sum + value;
        if self.sum.abs() >= value.abs() {
            self.compensation += (self.sum - next) + value;
        } else {
            self.compensation += (value - next) + self.sum;
        }
        self.sum = next;
    }
}

/// A fixed-capacity `f64` window with `O(1)` Welford variance updates.
///
/// The window keeps the Welford running mean and sum of squared deviations
/// `M2`, removing the evicted observation with the reverse Welford update and
/// adding the new one with the forward update:
///
/// ```text
/// add:    n <- n + 1; delta = x - mean; mean <- mean + delta / n; M2 <- M2 + delta * (x - mean)
/// remove: n <- n - 1; delta = x - mean; mean <- mean - delta / n; M2 <- M2 - delta * (x - mean)
/// ```
///
/// [`RollingVariance::variance`] reports the population variance `M2 / n`,
/// which is the convention used by Bollinger Bands. Both updates are `O(1)`
/// and allocation-free.
///
/// # Numerical note
///
/// Sliding Welford updates are stable for the window sizes used by technical
/// indicators, but they are not exactly compensated arithmetic. `M2` is
/// clamped at zero to absorb the tiny negative values that rounding can
/// produce for a constant window.
///
/// # Examples
///
/// ```
/// use kurumi_ta::RollingVariance;
///
/// # fn main() -> Result<(), kurumi_ta::KurumiError> {
/// let mut window = RollingVariance::new(3)?;
/// for value in [2.0, 4.0, 4.0] {
///     window.push(value)?;
/// }
/// assert_eq!(window.mean(), Some(10.0 / 3.0));
/// assert!((window.variance().expect("non-empty") - 8.0 / 9.0).abs() < 1e-12);
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone)]
pub struct RollingVariance {
    buffer: RingBuffer<f64>,
    count: usize,
    mean: f64,
    squared_deviations: f64,
}

impl RollingVariance {
    /// Creates an empty window with a fixed, positive capacity.
    ///
    /// # Errors
    ///
    /// Returns [`KurumiError::InvalidParameter`] when `capacity` is zero.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_ta::RollingVariance;
    ///
    /// # fn main() -> Result<(), kurumi_ta::KurumiError> {
    /// assert!(RollingVariance::new(0).is_err());
    /// assert_eq!(RollingVariance::new(3)?.capacity(), 3);
    /// # Ok(())
    /// # }
    /// ```
    pub fn new(capacity: usize) -> Result<Self, KurumiError> {
        Ok(Self {
            buffer: RingBuffer::new(capacity)?,
            count: 0,
            mean: 0.0,
            squared_deviations: 0.0,
        })
    }

    /// Pushes one finite observation, evicting the oldest when full.
    ///
    /// # Errors
    ///
    /// Returns [`KurumiError::InvalidParameter`] for non-finite observations.
    /// Returns [`KurumiError::NumericalOverflow`] if the running moments leave
    /// the finite range; the window is cleared before that error is returned.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_ta::RollingVariance;
    ///
    /// # fn main() -> Result<(), kurumi_ta::KurumiError> {
    /// let mut window = RollingVariance::new(2)?;
    /// window.push(1.0)?;
    /// window.push(3.0)?;
    /// assert_eq!(window.mean(), Some(2.0));
    /// assert!(window.push(f64::INFINITY).is_err());
    /// # Ok(())
    /// # }
    /// ```
    #[inline]
    pub fn push(&mut self, value: f64) -> Result<(), KurumiError> {
        ensure_finite(value, "value")?;

        if let Some(evicted) = self.buffer.push(value) {
            self.remove(evicted);
        }
        self.add(value);

        if !self.mean.is_finite() || !self.squared_deviations.is_finite() {
            self.clear();
            return Err(KurumiError::NumericalOverflow {
                operation: "RollingVariance::push",
            });
        }
        Ok(())
    }

    /// Returns the population mean of the live values, or `None` when empty.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_ta::RollingVariance;
    ///
    /// # fn main() -> Result<(), kurumi_ta::KurumiError> {
    /// let mut window = RollingVariance::new(3)?;
    /// assert_eq!(window.mean(), None);
    /// window.push(5.0)?;
    /// assert_eq!(window.mean(), Some(5.0));
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    #[inline]
    pub fn mean(&self) -> Option<f64> {
        if self.count == 0 {
            None
        } else {
            Some(self.mean)
        }
    }

    /// Returns the population variance of the live values, or `None` when
    /// empty.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_ta::RollingVariance;
    ///
    /// # fn main() -> Result<(), kurumi_ta::KurumiError> {
    /// let mut window = RollingVariance::new(2)?;
    /// window.push(1.0)?;
    /// window.push(3.0)?;
    /// assert_eq!(window.variance(), Some(1.0));
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    #[inline]
    pub fn variance(&self) -> Option<f64> {
        if self.count == 0 {
            None
        } else {
            Some((self.squared_deviations / self.count as f64).max(0.0))
        }
    }

    /// Returns the population standard deviation of the live values, or
    /// `None` when empty.
    ///
    /// # Examples
    ///
    /// ```
    /// use kurumi_ta::RollingVariance;
    ///
    /// # fn main() -> Result<(), kurumi_ta::KurumiError> {
    /// let mut window = RollingVariance::new(2)?;
    /// window.push(1.0)?;
    /// window.push(3.0)?;
    /// assert_eq!(window.std_dev(), Some(1.0));
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    #[inline]
    pub fn std_dev(&self) -> Option<f64> {
        self.variance().map(f64::sqrt)
    }

    /// Returns the number of live values.
    #[must_use]
    #[inline]
    pub fn len(&self) -> usize {
        self.count
    }

    /// Returns the fixed capacity.
    #[must_use]
    #[inline]
    pub const fn capacity(&self) -> usize {
        self.buffer.capacity()
    }

    /// Returns `true` when no value is stored.
    #[must_use]
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.count == 0
    }

    /// Returns `true` when the window is at capacity.
    #[must_use]
    #[inline]
    pub fn is_full(&self) -> bool {
        self.count == self.buffer.capacity()
    }

    /// Clears the values and moments while keeping the reserved capacity.
    pub fn clear(&mut self) {
        self.buffer.clear();
        self.count = 0;
        self.mean = 0.0;
        self.squared_deviations = 0.0;
    }

    /// Forward Welford update for one observation.
    #[inline]
    fn add(&mut self, value: f64) {
        self.count += 1;
        let delta = value - self.mean;
        self.mean += delta / self.count as f64;
        self.squared_deviations += delta * (value - self.mean);
    }

    /// Reverse Welford update for the observation leaving the window.
    #[inline]
    fn remove(&mut self, value: f64) {
        if self.count <= 1 {
            self.count = 0;
            self.mean = 0.0;
            self.squared_deviations = 0.0;
            return;
        }
        let remaining = self.count - 1;
        let delta = value - self.mean;
        self.mean -= delta / remaining as f64;
        self.squared_deviations -= delta * (value - self.mean);
        self.count = remaining;
        if self.squared_deviations < 0.0 {
            self.squared_deviations = 0.0;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use approx::assert_relative_eq;

    #[test]
    fn ring_buffer_evicts_in_fifo_order_and_wraps_repeatedly() {
        let mut buffer = RingBuffer::new(3).expect("positive capacity");
        assert_eq!(buffer.push(1), None);
        assert_eq!(buffer.push(2), None);
        assert_eq!(buffer.push(3), None);
        assert_eq!(buffer.push(4), Some(1));
        assert_eq!(buffer.push(5), Some(2));
        assert_eq!(buffer.push(6), Some(3));

        let live: Vec<_> = buffer.iter().copied().collect();
        assert_eq!(live, vec![4, 5, 6]);
        assert_eq!(buffer.front(), Some(&4));
        assert_eq!(buffer.back(), Some(&6));
        assert_eq!(buffer.get(1), Some(&5));
        assert_eq!(buffer.get(3), None);
    }

    #[test]
    fn ring_buffer_rejects_zero_capacity_and_reports_its_state() {
        assert!(matches!(
            RingBuffer::<f64>::new(0),
            Err(KurumiError::InvalidParameter { .. })
        ));
        let mut buffer = RingBuffer::new(2).expect("positive capacity");
        assert!(buffer.is_empty());
        assert!(!buffer.is_full());
        buffer.push(1.0);
        assert_eq!(buffer.len(), 1);
        assert_eq!(buffer.capacity(), 2);
        buffer.push(2.0);
        assert!(buffer.is_full());
        buffer.clear();
        assert!(buffer.is_empty());
        assert_eq!(buffer.capacity(), 2);
    }

    #[test]
    fn ring_buffer_reserves_its_full_capacity_up_front() {
        let buffer = RingBuffer::<f64>::new(8).expect("positive capacity");
        assert!(buffer.capacity() >= 8);
    }

    #[test]
    fn rolling_window_tracks_mean_min_and_max_incrementally() {
        let mut window = RollingWindow::new(3).expect("positive capacity");
        for value in [1.0, 4.0, 2.0] {
            window.push(value).expect("finite value");
        }
        assert_relative_eq!(
            window.mean().expect("non-empty"),
            7.0 / 3.0,
            epsilon = 1e-15
        );
        assert_eq!(window.min(), Some(1.0));
        assert_eq!(window.max(), Some(4.0));

        window.push(9.0).expect("finite value");
        assert_relative_eq!(window.mean().expect("non-empty"), 5.0, epsilon = 1e-15);
        assert_eq!(window.min(), Some(2.0));
        assert_eq!(window.max(), Some(9.0));

        window.push(0.0).expect("finite value");
        assert_relative_eq!(
            window.mean().expect("non-empty"),
            11.0 / 3.0,
            epsilon = 1e-15
        );
        assert_eq!(window.min(), Some(0.0));
        assert_eq!(window.max(), Some(9.0));
    }

    #[test]
    fn rolling_window_handles_duplicate_extremes_after_eviction() {
        let mut window = RollingWindow::new(3).expect("positive capacity");
        for value in [5.0, 5.0, 1.0] {
            window.push(value).expect("finite value");
        }
        assert_eq!(window.min(), Some(1.0));
        assert_eq!(window.max(), Some(5.0));

        window.push(5.0).expect("finite value");
        assert_eq!(window.max(), Some(5.0));
        window.push(2.0).expect("finite value");
        window.push(3.0).expect("finite value");
        assert_eq!(window.min(), Some(2.0));
        assert_eq!(window.max(), Some(5.0));
    }

    #[test]
    fn rolling_window_compensates_the_running_sum() {
        let mut window = RollingWindow::new(3).expect("positive capacity");
        window.push(1.0e16).expect("finite value");
        window.push(1.0).expect("finite value");
        window.push(-1.0e16).expect("finite value");
        assert_relative_eq!(
            window.mean().expect("non-empty"),
            1.0 / 3.0,
            epsilon = 1e-15
        );
    }

    #[test]
    fn rolling_window_rejects_non_finite_observations() {
        let mut window = RollingWindow::new(2).expect("positive capacity");
        window.push(1.0).expect("finite value");
        assert!(window.push(f64::NAN).is_err());
        assert!(window.push(f64::INFINITY).is_err());
        assert_eq!(window.len(), 1);
    }

    #[test]
    fn rolling_window_capacity_one_behaves_like_a_single_slot() {
        let mut window = RollingWindow::new(1).expect("positive capacity");
        window.push(3.0).expect("finite value");
        window.push(-2.0).expect("finite value");
        assert_eq!(window.mean(), Some(-2.0));
        assert_eq!(window.min(), Some(-2.0));
        assert_eq!(window.max(), Some(-2.0));
    }

    #[test]
    fn rolling_variance_matches_the_population_formula() {
        let mut window = RollingVariance::new(3).expect("positive capacity");
        for value in [2.0, 4.0, 4.0] {
            window.push(value).expect("finite value");
        }
        assert_relative_eq!(
            window.mean().expect("non-empty"),
            10.0 / 3.0,
            epsilon = 1e-15
        );
        assert_relative_eq!(
            window.variance().expect("non-empty"),
            8.0 / 9.0,
            epsilon = 1e-15
        );
        assert_relative_eq!(
            window.std_dev().expect("non-empty"),
            (8.0_f64 / 9.0).sqrt(),
            epsilon = 1e-15
        );
    }

    #[test]
    fn rolling_variance_removes_evicted_values_exactly() {
        let mut window = RollingVariance::new(3).expect("positive capacity");
        for value in [1.0, 2.0, 3.0] {
            window.push(value).expect("finite value");
        }
        window.push(7.0).expect("finite value");

        // Window is now [2, 3, 7]: mean 4, population variance 14 / 3.
        assert_relative_eq!(window.mean().expect("non-empty"), 4.0, epsilon = 1e-15);
        assert_relative_eq!(
            window.variance().expect("non-empty"),
            14.0 / 3.0,
            epsilon = 1e-15
        );
    }

    #[test]
    fn rolling_variance_of_a_constant_window_is_zero() {
        let mut window = RollingVariance::new(4).expect("positive capacity");
        for value in [7.0; 12] {
            window.push(value).expect("finite value");
        }
        assert_eq!(window.variance(), Some(0.0));
        assert_eq!(window.std_dev(), Some(0.0));
    }

    #[test]
    fn rolling_variance_reports_empty_and_full_states() {
        let mut window = RollingVariance::new(2).expect("positive capacity");
        assert!(window.is_empty());
        assert_eq!(window.mean(), None);
        assert_eq!(window.variance(), None);
        window.push(1.0).expect("finite value");
        assert!(!window.is_full());
        window.push(2.0).expect("finite value");
        assert!(window.is_full());
        assert_eq!(window.len(), 2);
        window.clear();
        assert!(window.is_empty());
        assert_eq!(window.capacity(), 2);
    }
}

//! Property-based equivalence between the streaming and batch forms.
//!
//! Every indicator is exercised on arbitrary finite series. The test computes a
//! naive, direct-from-slice reference with `O(N * window)` arithmetic and then
//! checks that
//!
//! 1. tick-by-tick [`StreamingIndicator::next`] calls, and
//! 2. one [`StreamingIndicator::compute_batch`] call
//!
//! both reproduce the reference within a relative tolerance. This is the
//! property the issue requires: the `O(1)` incremental state machine and the
//! vectorized batch entry point share one definition of the formula.

use kurumi_ta::{
    Atr, BollingerBands, BollingerBandsOutput, Ema, Macd, MacdOutput, OhlcvBar, Roc, Rsi, Sma,
    Stochastic, StochasticOutput, StreamingIndicator, Wma,
};
use proptest::prelude::*;

/// A random draw tolerance for `f64` accumulation drift. The incremental WMA
/// update can accumulate one-ulp-scale error when a window of large values
/// cancels to a near-zero mean, so the tolerance is one order of magnitude
/// above the machine-level relative error.
const TOLERANCE: f64 = 1e-8;

fn finite_prices() -> impl Strategy<Value = Vec<f64>> {
    prop::collection::vec(-1.0e6..1.0e6_f64, 1..160)
}

fn positive_prices() -> impl Strategy<Value = Vec<f64>> {
    prop::collection::vec(1.0e-3..1.0e6_f64, 1..160)
}

fn small_period() -> impl Strategy<Value = usize> {
    1usize..24
}

fn bar_strategy() -> impl Strategy<Value = OhlcvBar> {
    (
        any::<i64>(),
        0.5f64..1.0e6,
        0.5f64..1.0e6,
        0.5f64..1.0e6,
        0.5f64..1.0e6,
        0.0f64..1.0e6,
        0.0f64..1.0e6,
    )
        .prop_map(|(timestamp, a, b, c, d, volume, turnover)| {
            let high = a.max(b).max(c).max(d);
            let low = a.min(b).min(c).min(d);
            OhlcvBar::new(timestamp, a, high, low, d, volume, turnover)
                .expect("generated valid bar")
        })
}

fn bars() -> impl Strategy<Value = Vec<OhlcvBar>> {
    prop::collection::vec(bar_strategy(), 1..160)
}

/// Relative closeness for two optional indicator values. `None` represents the
/// warm-up period and must match exactly.
fn close(actual: Option<f64>, expected: Option<f64>) -> bool {
    match (actual, expected) {
        (None, None) => true,
        (Some(actual), Some(expected)) => {
            actual.is_finite() && (actual - expected).abs() <= TOLERANCE * expected.abs().max(1.0)
        }
        _ => false,
    }
}

fn direct_mean(values: &[f64]) -> f64 {
    values.iter().sum::<f64>() / values.len() as f64
}

fn naive_sma(values: &[f64], period: usize) -> Vec<Option<f64>> {
    values
        .iter()
        .enumerate()
        .map(|(index, _)| {
            if index + 1 >= period {
                Some(direct_mean(&values[index + 1 - period..=index]))
            } else {
                None
            }
        })
        .collect()
}

fn naive_ema(values: &[f64], period: usize) -> Vec<Option<f64>> {
    let mut output = vec![None; values.len()];
    if values.len() < period {
        return output;
    }
    let alpha = 2.0 / (period as f64 + 1.0);
    let mut value = direct_mean(&values[..period]);
    output[period - 1] = Some(value);
    for index in period..values.len() {
        value = alpha * values[index] + (1.0 - alpha) * value;
        output[index] = Some(value);
    }
    output
}

fn naive_wma(values: &[f64], period: usize) -> Vec<Option<f64>> {
    let denominator = period as f64 * (period as f64 + 1.0) / 2.0;
    values
        .iter()
        .enumerate()
        .map(|(index, _)| {
            if index + 1 < period {
                return None;
            }
            let window = &values[index + 1 - period..=index];
            let weighted = window
                .iter()
                .enumerate()
                .map(|(offset, value)| (offset + 1) as f64 * value)
                .sum::<f64>();
            Some(weighted / denominator)
        })
        .collect()
}

fn naive_macd(
    values: &[f64],
    fast_period: usize,
    slow_period: usize,
    signal_period: usize,
) -> Vec<Option<MacdOutput>> {
    let fast = naive_ema(values, fast_period);
    let slow = naive_ema(values, slow_period);
    let mut line = vec![None; values.len()];
    for index in 0..values.len() {
        if let (Some(fast), Some(slow)) = (fast[index], slow[index]) {
            line[index] = Some(fast - slow);
        }
    }

    let start = slow_period - 1;
    let mut signal = vec![None; values.len()];
    if values.len() >= start + signal_period {
        let seed_end = start + signal_period;
        let seeded = direct_mean(
            &line[start..seed_end]
                .iter()
                .map(|value| value.expect("line seeded with the slow EMA"))
                .collect::<Vec<_>>(),
        );
        signal[seed_end - 1] = Some(seeded);
        let alpha = 2.0 / (signal_period as f64 + 1.0);
        for index in seed_end..values.len() {
            let previous = signal[index - 1].expect("previous signal value");
            let macd = line[index].expect("line available with the signal");
            signal[index] = Some(alpha * macd + (1.0 - alpha) * previous);
        }
    }

    line.iter()
        .zip(signal.iter())
        .map(|(line, signal)| match (line, signal) {
            (Some(macd), Some(signal)) => Some(MacdOutput {
                macd: *macd,
                signal: *signal,
                histogram: macd - signal,
            }),
            _ => None,
        })
        .collect()
}

fn naive_rsi(values: &[f64], period: usize) -> Vec<Option<f64>> {
    let mut output = vec![None; values.len()];
    if values.len() < period + 1 {
        return output;
    }
    let mut seed_gain = 0.0;
    let mut seed_loss = 0.0;
    for index in 1..=period {
        let change = values[index] - values[index - 1];
        seed_gain += change.max(0.0);
        seed_loss += (-change).max(0.0);
    }
    let mut avg_gain = seed_gain / period as f64;
    let mut avg_loss = seed_loss / period as f64;
    output[period] = Some(rsi_value(avg_gain, avg_loss));
    for index in period + 1..values.len() {
        let change = values[index] - values[index - 1];
        let gain = change.max(0.0);
        let loss = (-change).max(0.0);
        avg_gain = (avg_gain * (period as f64 - 1.0) + gain) / period as f64;
        avg_loss = (avg_loss * (period as f64 - 1.0) + loss) / period as f64;
        output[index] = Some(rsi_value(avg_gain, avg_loss));
    }
    output
}

fn rsi_value(avg_gain: f64, avg_loss: f64) -> f64 {
    if avg_loss > 0.0 {
        100.0 - 100.0 / (1.0 + avg_gain / avg_loss)
    } else if avg_gain > 0.0 {
        100.0
    } else {
        50.0
    }
}

fn naive_stochastic(
    bars: &[OhlcvBar],
    k_period: usize,
    d_period: usize,
) -> Vec<Option<StochasticOutput>> {
    let mut output = vec![None; bars.len()];
    let mut k_values = Vec::new();
    for index in 0..bars.len() {
        if index + 1 < k_period {
            continue;
        }
        let window = &bars[index + 1 - k_period..=index];
        let highest = window
            .iter()
            .map(|bar| bar.high())
            .fold(f64::NEG_INFINITY, f64::max);
        let lowest = window
            .iter()
            .map(|bar| bar.low())
            .fold(f64::INFINITY, f64::min);
        let range = highest - lowest;
        let k = if range > 0.0 {
            100.0 * (bars[index].close() - lowest) / range
        } else {
            50.0
        };
        k_values.push(k);
        if k_values.len() >= d_period {
            let d = direct_mean(&k_values[k_values.len() - d_period..]);
            output[index] = Some(StochasticOutput { k, d });
        }
    }
    output
}

fn naive_roc(values: &[f64], period: usize) -> Vec<Option<f64>> {
    values
        .iter()
        .enumerate()
        .map(|(index, _)| {
            if index < period {
                return None;
            }
            let reference = values[index - period];
            Some(100.0 * (values[index] - reference) / reference)
        })
        .collect()
}

fn naive_atr(bars: &[OhlcvBar], period: usize) -> Vec<Option<f64>> {
    let mut output = vec![None; bars.len()];
    let mut true_ranges = Vec::new();
    let mut atr = 0.0;
    for index in 0..bars.len() {
        let true_range = if index == 0 {
            bars[index].high() - bars[index].low()
        } else {
            let previous_close = bars[index - 1].close();
            (bars[index].high() - bars[index].low())
                .max((bars[index].high() - previous_close).abs())
                .max((bars[index].low() - previous_close).abs())
        };
        true_ranges.push(true_range);
        if index + 1 < period {
            continue;
        }
        if index + 1 == period {
            atr = direct_mean(&true_ranges[..period]);
        } else {
            atr = (atr * (period as f64 - 1.0) + true_range) / period as f64;
        }
        output[index] = Some(atr);
    }
    output
}

fn naive_bollinger(
    values: &[f64],
    period: usize,
    multiplier: f64,
) -> Vec<Option<BollingerBandsOutput>> {
    values
        .iter()
        .enumerate()
        .map(|(index, _)| {
            if index + 1 < period {
                return None;
            }
            let window = &values[index + 1 - period..=index];
            let middle = direct_mean(window);
            let variance = window
                .iter()
                .map(|value| {
                    let deviation = value - middle;
                    deviation * deviation
                })
                .sum::<f64>()
                / period as f64;
            let sigma = variance.sqrt();
            Some(BollingerBandsOutput {
                middle,
                upper: middle + multiplier * sigma,
                lower: middle - multiplier * sigma,
            })
        })
        .collect()
}

fn tick_f64<I>(indicator: &mut I, values: &[f64]) -> Vec<Option<f64>>
where
    I: StreamingIndicator<Input = f64, Output = f64>,
{
    values
        .iter()
        .map(|&value| indicator.next(value).expect("finite input"))
        .collect()
}

fn check_f64_series(actual: &[Option<f64>], expected: &[Option<f64>], label: &str) -> bool {
    actual.len() == expected.len()
        && actual
            .iter()
            .zip(expected.iter())
            .all(|(actual, expected)| close(*actual, *expected))
        && !actual.is_empty()
        && !label.is_empty()
}

proptest! {
    #[test]
    fn sma_streaming_and_batch_match_the_direct_window_mean(
        values in finite_prices(),
        period in small_period(),
    ) {
        let expected = naive_sma(&values, period);
        let mut streaming = Sma::new(period).expect("positive period");
        let ticked = tick_f64(&mut streaming, &values);
        let mut batch = Sma::new(period).expect("positive period");
        let batched = batch.compute_batch(&values).expect("finite inputs");

        prop_assert!(check_f64_series(&ticked, &expected, "tick"));
        prop_assert!(check_f64_series(&batched, &expected, "batch"));
    }

    #[test]
    fn ema_streaming_and_batch_match_the_direct_recursion(
        values in finite_prices(),
        period in small_period(),
    ) {
        let expected = naive_ema(&values, period);
        let mut streaming = Ema::new(period).expect("positive period");
        let ticked = tick_f64(&mut streaming, &values);
        let mut batch = Ema::new(period).expect("positive period");
        let batched = batch.compute_batch(&values).expect("finite inputs");

        prop_assert!(check_f64_series(&ticked, &expected, "tick"));
        prop_assert!(check_f64_series(&batched, &expected, "batch"));
    }

    #[test]
    fn wma_streaming_and_batch_match_the_direct_weighted_sum(
        values in finite_prices(),
        period in small_period(),
    ) {
        let expected = naive_wma(&values, period);
        let mut streaming = Wma::new(period).expect("positive period");
        let ticked = tick_f64(&mut streaming, &values);
        let mut batch = Wma::new(period).expect("positive period");
        let batched = batch.compute_batch(&values).expect("finite inputs");

        prop_assert!(check_f64_series(&ticked, &expected, "tick"));
        prop_assert!(check_f64_series(&batched, &expected, "batch"));
    }

    #[test]
    fn macd_streaming_and_batch_match_the_direct_ema_chain(
        values in finite_prices(),
        fast_period in 1usize..12,
        extra in 1usize..12,
        signal_period in 1usize..8,
    ) {
        let slow_period = fast_period + extra;
        let expected = naive_macd(&values, fast_period, slow_period, signal_period);
        let mut streaming = Macd::new(fast_period, slow_period, signal_period).expect("valid periods");
        let ticked: Vec<_> = values
            .iter()
            .map(|&value| streaming.next(value).expect("finite input"))
            .collect();
        let mut batch = Macd::new(fast_period, slow_period, signal_period).expect("valid periods");
        let batched = batch.compute_batch(&values).expect("finite inputs");

        prop_assert_eq!(ticked.len(), expected.len());
        prop_assert_eq!(batched.len(), expected.len());
        for index in 0..expected.len() {
            match (ticked[index], expected[index]) {
                (None, None) => {}
                (Some(actual), Some(expected)) => {
                    prop_assert!(close(Some(actual.macd), Some(expected.macd)));
                    prop_assert!(close(Some(actual.signal), Some(expected.signal)));
                    prop_assert!(close(Some(actual.histogram), Some(expected.histogram)));
                }
                _ => prop_assert!(false, "warm-up mismatch at {}", index),
            }
            match (batched[index], expected[index]) {
                (None, None) => {}
                (Some(actual), Some(expected)) => {
                    prop_assert!(close(Some(actual.macd), Some(expected.macd)));
                    prop_assert!(close(Some(actual.signal), Some(expected.signal)));
                    prop_assert!(close(Some(actual.histogram), Some(expected.histogram)));
                }
                _ => prop_assert!(false, "batch warm-up mismatch at {}", index),
            }
        }
    }

    #[test]
    fn rsi_streaming_and_batch_match_wilder_smoothing(
        values in finite_prices(),
        period in small_period(),
    ) {
        let expected = naive_rsi(&values, period);
        let mut streaming = Rsi::new(period).expect("positive period");
        let ticked = tick_f64(&mut streaming, &values);
        let mut batch = Rsi::new(period).expect("positive period");
        let batched = batch.compute_batch(&values).expect("finite inputs");

        prop_assert!(check_f64_series(&ticked, &expected, "tick"));
        prop_assert!(check_f64_series(&batched, &expected, "batch"));
    }

    #[test]
    fn stochastic_streaming_and_batch_match_the_direct_range_and_sma(
        bars in bars(),
        k_period in 1usize..12,
        d_period in 1usize..12,
    ) {
        let expected = naive_stochastic(&bars, k_period, d_period);
        let mut streaming = Stochastic::new(k_period, d_period).expect("positive periods");
        let ticked: Vec<_> = bars
            .iter()
            .map(|&bar| streaming.next(bar).expect("valid bar"))
            .collect();
        let mut batch = Stochastic::new(k_period, d_period).expect("positive periods");
        let batched = batch.compute_batch(&bars).expect("valid bars");

        prop_assert_eq!(ticked.len(), expected.len());
        prop_assert_eq!(batched.len(), expected.len());
        for index in 0..expected.len() {
            match (ticked[index], expected[index]) {
                (None, None) => {}
                (Some(actual), Some(expected)) => {
                    prop_assert!(close(Some(actual.k), Some(expected.k)));
                    prop_assert!(close(Some(actual.d), Some(expected.d)));
                }
                _ => prop_assert!(false, "warm-up mismatch at {}", index),
            }
            match (batched[index], expected[index]) {
                (None, None) => {}
                (Some(actual), Some(expected)) => {
                    prop_assert!(close(Some(actual.k), Some(expected.k)));
                    prop_assert!(close(Some(actual.d), Some(expected.d)));
                }
                _ => prop_assert!(false, "batch warm-up mismatch at {}", index),
            }
        }
    }

    #[test]
    fn roc_streaming_and_batch_match_the_direct_percentage_change(
        values in positive_prices(),
        period in small_period(),
    ) {
        let expected = naive_roc(&values, period);
        let mut streaming = Roc::new(period).expect("positive period");
        let ticked = tick_f64(&mut streaming, &values);
        let mut batch = Roc::new(period).expect("positive period");
        let batched = batch.compute_batch(&values).expect("finite inputs");

        prop_assert!(check_f64_series(&ticked, &expected, "tick"));
        prop_assert!(check_f64_series(&batched, &expected, "batch"));
    }

    #[test]
    fn atr_streaming_and_batch_match_the_direct_true_range_average(
        bars in bars(),
        period in small_period(),
    ) {
        let expected = naive_atr(&bars, period);
        let mut streaming = Atr::new(period).expect("positive period");
        let ticked: Vec<_> = bars
            .iter()
            .map(|&bar| streaming.next(bar).expect("valid bar"))
            .collect();
        let mut batch = Atr::new(period).expect("positive period");
        let batched = batch.compute_batch(&bars).expect("valid bars");
        prop_assert!(check_f64_series(&ticked, &expected, "tick"));
        prop_assert!(check_f64_series(&batched, &expected, "batch"));
    }

    #[test]
    fn bollinger_streaming_and_batch_match_the_direct_population_variance(
        values in positive_prices(),
        period in small_period(),
        multiplier in 0.0f64..4.0,
    ) {
        let expected = naive_bollinger(&values, period, multiplier);
        let mut streaming = BollingerBands::new(period, multiplier).expect("valid parameters");
        let ticked: Vec<_> = values
            .iter()
            .map(|&value| streaming.next(value).expect("finite input"))
            .collect();
        let mut batch = BollingerBands::new(period, multiplier).expect("valid parameters");
        let batched = batch.compute_batch(&values).expect("finite inputs");

        prop_assert_eq!(ticked.len(), expected.len());
        prop_assert_eq!(batched.len(), expected.len());
        for index in 0..expected.len() {
            match (ticked[index], expected[index]) {
                (None, None) => {}
                (Some(actual), Some(expected)) => {
                    prop_assert!(close(Some(actual.middle), Some(expected.middle)));
                    prop_assert!(close(Some(actual.upper), Some(expected.upper)));
                    prop_assert!(close(Some(actual.lower), Some(expected.lower)));
                }
                _ => prop_assert!(false, "warm-up mismatch at {}", index),
            }
            match (batched[index], expected[index]) {
                (None, None) => {}
                (Some(actual), Some(expected)) => {
                    prop_assert!(close(Some(actual.middle), Some(expected.middle)));
                    prop_assert!(close(Some(actual.upper), Some(expected.upper)));
                    prop_assert!(close(Some(actual.lower), Some(expected.lower)));
                }
                _ => prop_assert!(false, "batch warm-up mismatch at {}", index),
            }
        }
    }
}

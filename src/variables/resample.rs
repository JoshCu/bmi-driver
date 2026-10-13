use crate::config::{DownsampleMode, UpsampleMode};
use crate::error::{function_failed, BmiResult};
use std::collections::HashMap;

/// Timesteps closer than this (in seconds) are treated as equal.
pub const TIMESTEP_EPSILON: f64 = 1e-9;

fn err(msg: String) -> crate::error::BmiError {
    function_failed("resample", msg)
}

/// Resample a single value from a source time series to a destination time.
///
/// - `source_values`: complete output array from the source (one value per source step)
/// - `source_dt`: source timestep in seconds
/// - `dest_time`: target time in seconds from simulation start
/// - `dest_dt`: destination timestep in seconds
/// - `downsample_mode`: how to handle coarser source → finer dest
/// - `upsample_mode`: how to handle finer source → coarser dest
///
/// Generic over the element type so `f32` forcing series are read in place, not copied to `f64`.
pub fn resample_value<T: Copy + Into<f64>>(
    source_values: &[T],
    source_dt: f64,
    dest_time: f64,
    dest_dt: f64,
    downsample_mode: DownsampleMode,
    upsample_mode: UpsampleMode,
) -> BmiResult<f64> {
    if source_values.is_empty() {
        return Err(err("empty source values".into()));
    }

    let ratio = source_dt / dest_dt;

    if (ratio - 1.0).abs() < TIMESTEP_EPSILON {
        // Same timestep: direct index
        let idx = (dest_time / source_dt).round() as usize;
        return source_values.get(idx).map(|&v| v.into()).ok_or_else(|| {
            err(format!(
                "index {} out of bounds (len {})",
                idx,
                source_values.len()
            ))
        });
    }

    if ratio > 1.0 {
        // Source is coarser than dest (downsample: e.g., daily source → hourly dest)
        downsample(source_values, source_dt, dest_time, downsample_mode)
    } else {
        // Source is finer than dest (upsample: e.g., 5min source → hourly dest)
        upsample(source_values, source_dt, dest_time, dest_dt, upsample_mode)
    }
}

/// Downsample: source has larger timestep than destination.
/// We need to produce more values from fewer source values.
fn downsample<T: Copy + Into<f64>>(
    source_values: &[T],
    source_dt: f64,
    dest_time: f64,
    mode: DownsampleMode,
) -> BmiResult<f64> {
    let fractional_idx = dest_time / source_dt;
    let lower_idx = fractional_idx.floor() as usize;

    match mode {
        DownsampleMode::Repeat => {
            let idx = lower_idx.min(source_values.len() - 1);
            Ok(source_values[idx].into())
        }
        DownsampleMode::Interpolate => {
            if lower_idx >= source_values.len() {
                return Ok((*source_values.last().unwrap()).into());
            }
            let frac = fractional_idx - lower_idx as f64;
            let lower_val: f64 = source_values[lower_idx].into();
            let upper_idx = lower_idx + 1;
            if upper_idx >= source_values.len() || frac.abs() < 1e-12 {
                return Ok(lower_val);
            }
            let upper_val: f64 = source_values[upper_idx].into();
            Ok(lower_val + frac * (upper_val - lower_val))
        }
    }
}

/// Upsample: source has smaller timestep than destination.
/// We need to aggregate multiple source values into one.
fn upsample<T: Copy + Into<f64>>(
    source_values: &[T],
    source_dt: f64,
    dest_time: f64,
    dest_dt: f64,
    mode: UpsampleMode,
) -> BmiResult<f64> {
    let window_start = dest_time;
    let window_end = dest_time + dest_dt;

    let start_idx = (window_start / source_dt).floor() as usize;
    let end_idx = ((window_end / source_dt).ceil() as usize).min(source_values.len());

    if start_idx >= source_values.len() {
        return Ok((*source_values.last().unwrap()).into());
    }

    let window = &source_values[start_idx..end_idx];
    if window.is_empty() {
        return Ok((*source_values.last().unwrap()).into());
    }

    aggregate(window, mode)
}

/// Resample a forcing series. Same as [`resample_value`] except at the end of the data: a
/// timestep the forcings don't cover is an error here, where model outputs clamp to the last
/// value. `name` is only used in error messages.
pub fn resample_forcing<T: Copy + Into<f64>>(
    name: &str,
    source_values: &[T],
    source_dt: f64,
    dest_time: f64,
    dest_dt: f64,
    downsample_mode: DownsampleMode,
    upsample_mode: UpsampleMode,
) -> BmiResult<f64> {
    let step = (dest_time / source_dt).floor() as usize;
    if step >= source_values.len() {
        return Err(err(format!(
            "forcing '{}' not available at step {}",
            name, step
        )));
    }
    resample_value(
        source_values,
        source_dt,
        dest_time,
        dest_dt,
        downsample_mode,
        upsample_mode,
    )
}

/// Aggregate a slice of values using the given mode.
pub fn aggregate<T: Copy + Into<f64>>(values: &[T], mode: UpsampleMode) -> BmiResult<f64> {
    if values.is_empty() {
        return Err(err("empty values for aggregation".into()));
    }
    let n = values.len();
    let values = values.iter().map(|&v| v.into());

    match mode {
        UpsampleMode::Mean => Ok(values.sum::<f64>() / n as f64),
        UpsampleMode::Min => Ok(values.fold(f64::INFINITY, f64::min)),
        UpsampleMode::Max => Ok(values.fold(f64::NEG_INFINITY, f64::max)),
        UpsampleMode::Mode => {
            let mut counts: HashMap<i64, usize> = HashMap::new();
            for v in values {
                *counts.entry(v.round() as i64).or_insert(0) += 1;
            }
            let best = counts.into_iter().max_by_key(|&(_, c)| c).unwrap();
            Ok(best.0 as f64)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_same_timestep_passthrough() {
        let source = vec![1.0, 2.0, 3.0, 4.0];
        let dt = 3600.0;
        for i in 0..4 {
            let val = resample_value(
                &source,
                dt,
                i as f64 * dt,
                dt,
                DownsampleMode::Repeat,
                UpsampleMode::Mean,
            )
            .unwrap();
            assert!((val - source[i]).abs() < 1e-12);
        }
    }

    #[test]
    fn test_downsample_repeat() {
        // Hourly source (3600s), 15-min dest (900s)
        let source = vec![10.0, 20.0, 30.0];
        let source_dt = 3600.0;
        let dest_dt = 900.0;

        // At dest_time=0, 900, 1800, 2700 → all map to source[0]=10
        for t in [0.0, 900.0, 1800.0, 2700.0] {
            let val = resample_value(
                &source,
                source_dt,
                t,
                dest_dt,
                DownsampleMode::Repeat,
                UpsampleMode::Mean,
            )
            .unwrap();
            assert!((val - 10.0).abs() < 1e-12, "at t={}, got {}", t, val);
        }
        // At dest_time=3600 → source[1]=20
        let val = resample_value(
            &source,
            source_dt,
            3600.0,
            dest_dt,
            DownsampleMode::Repeat,
            UpsampleMode::Mean,
        )
        .unwrap();
        assert!((val - 20.0).abs() < 1e-12);
    }

    #[test]
    fn test_downsample_interpolate() {
        // Hourly source (3600s), 15-min dest (900s)
        let source = vec![10.0, 20.0, 30.0];
        let source_dt = 3600.0;
        let dest_dt = 900.0;

        // At t=0 → 10.0
        let val = resample_value(
            &source,
            source_dt,
            0.0,
            dest_dt,
            DownsampleMode::Interpolate,
            UpsampleMode::Mean,
        )
        .unwrap();
        assert!((val - 10.0).abs() < 1e-12);

        // At t=1800 (half between source[0] and source[1]) → 15.0
        let val = resample_value(
            &source,
            source_dt,
            1800.0,
            dest_dt,
            DownsampleMode::Interpolate,
            UpsampleMode::Mean,
        )
        .unwrap();
        assert!((val - 15.0).abs() < 1e-12);

        // At t=900 (quarter) → 12.5
        let val = resample_value(
            &source,
            source_dt,
            900.0,
            dest_dt,
            DownsampleMode::Interpolate,
            UpsampleMode::Mean,
        )
        .unwrap();
        assert!((val - 12.5).abs() < 1e-12);

        // At t=3600 → 20.0
        let val = resample_value(
            &source,
            source_dt,
            3600.0,
            dest_dt,
            DownsampleMode::Interpolate,
            UpsampleMode::Mean,
        )
        .unwrap();
        assert!((val - 20.0).abs() < 1e-12);
    }

    #[test]
    fn test_upsample_mean() {
        // 15-min source (900s), hourly dest (3600s)
        let source = vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0];
        let source_dt = 900.0;
        let dest_dt = 3600.0;

        // At t=0, window [0, 3600) → indices 0..4 → [1,2,3,4] → mean=2.5
        let val = resample_value(
            &source,
            source_dt,
            0.0,
            dest_dt,
            DownsampleMode::Repeat,
            UpsampleMode::Mean,
        )
        .unwrap();
        assert!((val - 2.5).abs() < 1e-12);

        // At t=3600, window [3600, 7200) → indices 4..8 → [5,6,7,8] → mean=6.5
        let val = resample_value(
            &source,
            source_dt,
            3600.0,
            dest_dt,
            DownsampleMode::Repeat,
            UpsampleMode::Mean,
        )
        .unwrap();
        assert!((val - 6.5).abs() < 1e-12);
    }

    #[test]
    fn test_upsample_min_max() {
        let source = vec![3.0, 1.0, 4.0, 1.0, 5.0, 9.0, 2.0, 6.0];
        let source_dt = 900.0;
        let dest_dt = 3600.0;

        // Window [0, 3600) → [3, 1, 4, 1]
        let val = resample_value(
            &source,
            source_dt,
            0.0,
            dest_dt,
            DownsampleMode::Repeat,
            UpsampleMode::Min,
        )
        .unwrap();
        assert!((val - 1.0).abs() < 1e-12);

        let val = resample_value(
            &source,
            source_dt,
            0.0,
            dest_dt,
            DownsampleMode::Repeat,
            UpsampleMode::Max,
        )
        .unwrap();
        assert!((val - 4.0).abs() < 1e-12);

        // Window [3600, 7200) → [5, 9, 2, 6]
        let val = resample_value(
            &source,
            source_dt,
            3600.0,
            dest_dt,
            DownsampleMode::Repeat,
            UpsampleMode::Min,
        )
        .unwrap();
        assert!((val - 2.0).abs() < 1e-12);

        let val = resample_value(
            &source,
            source_dt,
            3600.0,
            dest_dt,
            DownsampleMode::Repeat,
            UpsampleMode::Max,
        )
        .unwrap();
        assert!((val - 9.0).abs() < 1e-12);
    }

    #[test]
    fn test_upsample_mode() {
        let source = vec![1.0, 2.0, 2.0, 3.0]; // mode is 2
        let source_dt = 900.0;
        let dest_dt = 3600.0;

        let val = resample_value(
            &source,
            source_dt,
            0.0,
            dest_dt,
            DownsampleMode::Repeat,
            UpsampleMode::Mode,
        )
        .unwrap();
        assert!((val - 2.0).abs() < 1e-12);
    }

    #[test]
    fn test_boundary_last_step_interpolate() {
        let source = vec![10.0, 20.0];
        let source_dt = 3600.0;
        let dest_dt = 900.0;

        // At the very last source time, should return last value
        let val = resample_value(
            &source,
            source_dt,
            3600.0,
            dest_dt,
            DownsampleMode::Interpolate,
            UpsampleMode::Mean,
        )
        .unwrap();
        assert!((val - 20.0).abs() < 1e-12);

        // Past the last source, should clamp to last
        let val = resample_value(
            &source,
            source_dt,
            5400.0,
            dest_dt,
            DownsampleMode::Interpolate,
            UpsampleMode::Mean,
        )
        .unwrap();
        assert!((val - 20.0).abs() < 1e-12);
    }

    #[test]
    fn test_non_integer_ratio() {
        // 7-min source (420s), hourly dest (3600s)
        // ratio = 420/3600 ≈ 0.117 → upsample
        let source: Vec<f64> = (0..60).map(|i| i as f64).collect();
        let source_dt = 420.0;
        let dest_dt = 3600.0;

        // Window [0, 3600) → indices 0..ceil(3600/420)=9 → [0..8]
        let val = resample_value(
            &source,
            source_dt,
            0.0,
            dest_dt,
            DownsampleMode::Repeat,
            UpsampleMode::Mean,
        )
        .unwrap();
        // indices 0..9 → values 0,1,2,3,4,5,6,7,8 → mean = 4.0
        assert!((val - 4.0).abs() < 1e-12, "got {}", val);
    }

    #[test]
    fn test_aggregate_empty() {
        let result = aggregate::<f64>(&[], UpsampleMode::Mean);
        assert!(result.is_err());
    }

    fn forcing(
        values: &[f32],
        source_dt: f64,
        dest_time: f64,
        dest_dt: f64,
        down: DownsampleMode,
    ) -> BmiResult<f64> {
        resample_forcing(
            "f",
            values,
            source_dt,
            dest_time,
            dest_dt,
            down,
            UpsampleMode::Mean,
        )
    }

    #[test]
    fn test_forcing_reads_f32_in_place() {
        let v = [1.5f32, 2.25, 0.1];
        for (i, &x) in v.iter().enumerate() {
            let got = forcing(
                &v,
                3600.0,
                i as f64 * 3600.0,
                3600.0,
                DownsampleMode::Repeat,
            );
            assert_eq!(got.unwrap(), x as f64);
        }
    }

    #[test]
    fn test_forcing_downsample_and_upsample() {
        let v = [0.0f32, 10.0, 20.0, 30.0];
        let interp = forcing(&v, 3600.0, 1800.0, 1800.0, DownsampleMode::Interpolate);
        assert_eq!(interp.unwrap(), 5.0);
        let repeat = forcing(&v, 3600.0, 5400.0, 1800.0, DownsampleMode::Repeat);
        assert_eq!(repeat.unwrap(), 10.0);
        // Two-hour window over hourly data: mean of 10 and 20.
        let up = forcing(&v, 3600.0, 3600.0, 7200.0, DownsampleMode::Repeat);
        assert_eq!(up.unwrap(), 15.0);
        // A window that runs off the end aggregates what is there.
        let tail = forcing(&v, 3600.0, 3.0 * 3600.0, 7200.0, DownsampleMode::Repeat);
        assert_eq!(tail.unwrap(), 30.0);
    }

    #[test]
    fn test_forcing_errors_past_end_of_data() {
        // Unlike model outputs, forcings don't clamp to their last value.
        let v = [1.0f32, 2.0];
        assert!(forcing(&v, 3600.0, 2.0 * 3600.0, 3600.0, DownsampleMode::Repeat).is_err());
        assert!(forcing(&v, 3600.0, 4.0 * 3600.0, 1800.0, DownsampleMode::Repeat).is_err());
        assert!(forcing(&v, 3600.0, 2.0 * 3600.0, 7200.0, DownsampleMode::Repeat).is_err());
        // Interpolating toward a missing next value keeps the last one.
        let last = forcing(&v, 3600.0, 5400.0, 1800.0, DownsampleMode::Interpolate);
        assert_eq!(last.unwrap(), 2.0);
        // ...as model outputs do throughout.
        let out = resample_value(
            &[1.0f64, 2.0],
            3600.0,
            4.0 * 3600.0,
            1800.0,
            DownsampleMode::Repeat,
            UpsampleMode::Mean,
        );
        assert_eq!(out.unwrap(), 2.0);
    }
}

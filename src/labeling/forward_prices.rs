//! Forward mid-price trajectory computation.
//!
//! Exports `[N_bins, max_horizon + 1]` f64 USD mid-price trajectories.
//! Column 0 = base price at time t. Column H = price H bins ahead.
//!
//! Source: docs/design/04_FEATURE_SPECIFICATION.md §6

use crate::contract::EPS;

/// Computes forward mid-price trajectories for export.
pub struct ForwardPriceComputer {
    max_horizon: usize,
}

impl ForwardPriceComputer {
    /// Create a new forward price computer.
    ///
    /// # Arguments
    ///
    /// * `max_horizon` — Maximum horizon in bins. Determines the number of
    ///   columns: `max_horizon + 1` (column 0 = base price).
    pub fn new(max_horizon: usize) -> Self {
        Self { max_horizon }
    }

    /// Number of columns in the output: `max_horizon + 1`.
    pub fn n_columns(&self) -> usize {
        self.max_horizon + 1
    }

    /// Compute forward price trajectories for all bins.
    ///
    /// `forward_prices[t][k] = mid_price[t + k]` for `k = 0..=max_horizon`.
    /// NaN where `t + k >= mid_prices.len()` or `mid_prices[t+k].abs() < EPS`.
    ///
    /// # Arguments
    ///
    /// * `mid_prices` — BBO midpoints at END of each post-warmup bin (f64 USD)
    ///
    /// # Returns
    ///
    /// Vec of rows, each with `max_horizon + 1` columns (f64 USD).
    pub fn compute(&self, mid_prices: &[f64]) -> Vec<Vec<f64>> {
        let n_bins = mid_prices.len();
        let n_cols = self.n_columns();
        let mut result = Vec::with_capacity(n_bins);

        for t in 0..n_bins {
            let mut row = Vec::with_capacity(n_cols);
            for k in 0..n_cols {
                if t + k < n_bins && mid_prices[t + k].abs() > EPS && mid_prices[t + k].is_finite()
                {
                    row.push(mid_prices[t + k]);
                } else {
                    row.push(f64::NAN);
                }
            }
            result.push(row);
        }

        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_forward_prices_shape() {
        let fpc = ForwardPriceComputer::new(5);
        let prices = vec![100.0, 101.0, 102.0, 103.0, 104.0, 105.0, 106.0];
        let fwd = fpc.compute(&prices);

        assert_eq!(fwd.len(), 7, "One row per bin");
        for row in &fwd {
            assert_eq!(row.len(), 6, "max_horizon + 1 = 6 columns");
        }
    }

    #[test]
    fn test_forward_prices_column_zero_is_base() {
        let fpc = ForwardPriceComputer::new(3);
        let prices = vec![100.0, 101.0, 102.0, 103.0, 104.0];
        let fwd = fpc.compute(&prices);

        for (t, row) in fwd.iter().enumerate() {
            assert_eq!(row[0], prices[t], "Column 0 = base price at bin {}", t);
        }
    }

    #[test]
    fn test_forward_prices_nan_at_end() {
        let fpc = ForwardPriceComputer::new(3);
        let prices = vec![100.0, 101.0, 102.0];
        let fwd = fpc.compute(&prices);

        // Bin 0: cols 0,1,2 valid, col 3 NaN (t+3 = 3 >= 3)
        assert!(fwd[0][0].is_finite());
        assert!(fwd[0][1].is_finite());
        assert!(fwd[0][2].is_finite());
        assert!(fwd[0][3].is_nan());

        // Bin 2: only col 0 valid
        assert!(fwd[2][0].is_finite());
        assert!(fwd[2][1].is_nan());
    }

    #[test]
    fn test_forward_prices_values_match() {
        let fpc = ForwardPriceComputer::new(2);
        let prices = vec![100.0, 101.0, 102.0, 103.0];
        let fwd = fpc.compute(&prices);

        assert_eq!(fwd[0][0], 100.0);
        assert_eq!(fwd[0][1], 101.0);
        assert_eq!(fwd[0][2], 102.0);
        assert_eq!(fwd[1][0], 101.0);
        assert_eq!(fwd[1][1], 102.0);
        assert_eq!(fwd[1][2], 103.0);
    }

    #[test]
    fn test_forward_prices_empty_input() {
        let fpc = ForwardPriceComputer::new(5);
        let fwd = fpc.compute(&[]);
        assert!(fwd.is_empty());
    }

    #[test]
    fn test_forward_prices_zero_midprice_is_nan() {
        let fpc = ForwardPriceComputer::new(2);
        let prices = vec![100.0, 0.0, 102.0];
        let fwd = fpc.compute(&prices);

        // Column 1 at bin 0 should be NaN (mid[1] = 0.0 < EPS)
        assert!(fwd[0][1].is_nan(), "Zero mid-price should produce NaN");
        // Column 0 at bin 1 should be NaN (mid[1] = 0.0)
        assert!(fwd[1][0].is_nan());
    }

    #[test]
    fn test_n_columns() {
        assert_eq!(ForwardPriceComputer::new(60).n_columns(), 61);
        assert_eq!(ForwardPriceComputer::new(0).n_columns(), 1);
    }
}

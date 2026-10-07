//! Forecast scores shared by the offline tuners and the model evaluation.

use crate::{Outcome, bayesian::Probabilities};
use serde::Serialize;

/// Arithmetic mean; `None` when there are no values.
pub fn mean(values: &[f64]) -> Option<f64> {
    (!values.is_empty()).then(|| values.iter().sum::<f64>() / values.len() as f64)
}

/// Sample standard deviation divided by the square root of the count; `None` below two values.
pub fn standard_error(values: &[f64]) -> Option<f64> {
    (values.len() >= 2).then(|| {
        let n = values.len() as f64;
        let mean = values.iter().sum::<f64>() / n;
        (values.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / (n - 1.0) / n).sqrt()
    })
}

/// Natural-log loss of the observed outcome, with its probability floored at 1e-15.
pub fn log_loss(p: Probabilities, outcome: &Outcome) -> f64 {
    -p.for_outcome(outcome).max(1e-15).ln()
}

/// Brier score summed over all three outcomes.
pub fn brier(p: Probabilities, outcome: &Outcome) -> f64 {
    Outcome::ALL
        .iter()
        .map(|o| (p.for_outcome(o) - f64::from(o == outcome)).powi(2))
        .sum()
}

/// Forecasts whose probability of `outcome` fell in `[lower, upper)`; the top bin also holds probability 1.
#[derive(Clone, Serialize)]
pub struct CalibrationBin {
    pub outcome: Outcome,
    pub lower: f64,
    pub upper: f64,
    pub games: usize,
    pub mean_probability: f64,
    pub observed_rate: f64,
}

/// Ten equal-width probability bins per outcome, omitting empty bins. `forecast` gives each row's probabilities
/// and observed outcome.
pub fn calibration<T>(rows: &[T], forecast: impl Fn(&T) -> (Probabilities, &Outcome)) -> Vec<CalibrationBin> {
    let mut calibration = Vec::new();
    for outcome in Outcome::ALL {
        let mut bins = [(0_usize, 0.0, 0_usize); 10];
        for row in rows {
            let (probabilities, observed) = forecast(row);
            let p = probabilities.for_outcome(&outcome);
            let bin = &mut bins[((p * 10.0) as usize).min(9)];
            bin.0 += 1;
            bin.1 += p;
            bin.2 += usize::from(*observed == outcome);
        }
        for (i, (games, total, observed)) in bins.into_iter().enumerate().filter(|(_, b)| b.0 > 0) {
            calibration.push(CalibrationBin {
                outcome: outcome.clone(),
                lower: i as f64 / 10.0,
                upper: (i + 1) as f64 / 10.0,
                games,
                mean_probability: total / games as f64,
                observed_rate: observed as f64 / games as f64,
            });
        }
    }
    calibration
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(home_win: f64, away_win: f64, tie: f64) -> Probabilities {
        Probabilities { home_win, away_win, tie }
    }

    #[test]
    fn scores_the_observed_outcome_and_all_three_squared_errors() {
        assert!((log_loss(p(0.5, 0.3, 0.2), &Outcome::Tie) + 0.2_f64.ln()).abs() < 1e-12);
        assert!((log_loss(p(1.0, 0.0, 0.0), &Outcome::AwayWin) + 1e-15_f64.ln()).abs() < 1e-12);
        assert!((brier(p(0.5, 0.3, 0.2), &Outcome::Tie) - 0.98).abs() < 1e-12);
        assert_eq!(standard_error(&[1.0, 3.0]), Some(1.0));
        assert_eq!(standard_error(&[1.0]), None);
        assert_eq!(mean(&[1.0, 2.0]), Some(1.5));
        assert_eq!(mean(&[]), None);
    }

    #[test]
    fn calibration_bins_each_outcome_and_skips_empty_bins() {
        let rows = [
            (p(0.62, 0.38, 0.0), Outcome::HomeWin),
            (p(0.68, 0.32, 0.0), Outcome::AwayWin),
            (p(1.0, 0.0, 0.0), Outcome::HomeWin),
        ];
        let bins = calibration(&rows, |(p, o)| (*p, o));
        let home: Vec<_> = bins.iter().filter(|b| b.outcome == Outcome::HomeWin).collect();
        assert_eq!(home.len(), 2);
        assert_eq!((home[0].lower, home[0].games, home[0].observed_rate), (0.6, 2, 0.5));
        assert!((home[0].mean_probability - 0.65).abs() < 1e-12);
        assert_eq!((home[1].lower, home[1].upper, home[1].games), (0.9, 1.0, 1));
        assert_eq!(
            bins.iter()
                .filter(|b| b.outcome == Outcome::Tie)
                .map(|b| b.games)
                .sum::<usize>(),
            3
        );
    }
}

#[cfg(feature = "codegen")]
use proc_macro2::TokenStream;
#[cfg(feature = "codegen")]
use quote::{ToTokens, quote};
use serde::Deserialize;

use super::int_provider::IntProvider;

#[derive(Deserialize, Clone, Debug)]
pub struct Experience {
    /// The experience points, represented as an `IntProvider`.
    pub experience: IntProvider,
}

#[cfg(feature = "codegen")]
impl ToTokens for Experience {
    fn to_tokens(&self, tokens: &mut TokenStream) {
        let experience = self.experience.to_token_stream();

        tokens.extend(quote! {
            Experience { experience: #experience }
        });
    }
}

/// Returns the number of points required to progress within a specific level.
///
/// # Arguments
/// * `level` – The level to calculate points for.
#[must_use]
pub const fn points_in_level(level: i32) -> i32 {
    match level {
        0..=15 => 2 * level + 7,
        16..=30 => 5 * level - 38,
        _ => 9 * level - 158,
    }
}

/// Calculates the total points required to reach a given level.
///
/// # Arguments
/// * `level` – The target level.
#[must_use]
pub fn points_to_level(level: i32) -> i32 {
    match level {
        0..=16 => level * level + 6 * level,
        17..=31 => {
            (2.5f64.mul_add(f64::from(level * level), -(40.5 * f64::from(level))) + 360.0) as i32
        }
        _ => {
            (4.5f64.mul_add(f64::from(level * level), -(162.5 * f64::from(level))) + 2220.0) as i32
        }
    }
}

/// Converts total experience points into a level and points within that level.
///
/// # Arguments
/// * `total_points` – The total accumulated experience points.
///
/// # Returns
/// A tuple `(level, points_into_level)` representing the current level and
/// remaining points within that level.
#[must_use]
pub fn total_to_level_and_points(total_points: i32) -> (i32, i32) {
    let estimate = match total_points {
        0..=352 => ((f64::from(total_points) + 9.0).sqrt() - 3.0) as i32,
        353..=1507 => (8.1 + (0.4 * (f64::from(total_points) - (7839.0 / 40.0))).sqrt()) as i32,
        _ => {
            ((325.0 / 18.0) + (2.0 / 9.0 * (f64::from(total_points) - (54215.0 / 72.0))).sqrt())
                as i32
        }
    };
    // The square roots above can land just under a whole level when the total is exactly
    // a level's start (10000 levels gives 9999.999...), so settle the level on exact points.
    let mut level = estimate.max(0);
    while level > 0 && points_to_level(level) > total_points {
        level -= 1;
    }
    while points_to_level(level + 1) <= total_points {
        level += 1;
    }
    let level_start = points_to_level(level);
    let points_into_level = total_points - level_start;

    (level, points_into_level)
}

/// Calculates the progress within a level as a value between 0.0 and 1.0.
///
/// # Arguments
/// * `points` – The points accumulated in the current level.
/// * `level` – The current level.
#[must_use]
pub fn progress_in_level(points: i32, level: i32) -> f32 {
    let max_points = points_in_level(level);
    let progress = (points as f32) / (max_points as f32);

    progress.clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::{points_in_level, points_to_level, total_to_level_and_points};

    #[test]
    fn total_points_at_a_level_start_give_that_level() {
        for level in [0, 1, 15, 16, 17, 30, 31, 32, 100, 9_999, 10_000, 20_000] {
            assert_eq!(
                total_to_level_and_points(points_to_level(level)),
                (level, 0),
                "start of level {level}"
            );
        }
    }

    #[test]
    fn total_points_just_under_the_next_level_stay_on_the_level() {
        for level in [0, 15, 16, 30, 31, 9_999, 10_000] {
            let last_point = points_to_level(level) + points_in_level(level) - 1;
            assert_eq!(
                total_to_level_and_points(last_point),
                (level, points_in_level(level) - 1),
                "end of level {level}"
            );
        }
    }
}

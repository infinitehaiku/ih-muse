// crates/ih-muse/src/tasks/interval.rs

use tokio::time::Duration;

use ih_muse_proto::TimestampResolution;

/// Calculates the interval duration based on the `finest_resolution` and a divisor.
/// Ensures the duration is at least 1 nanosecond.
///
/// # Arguments
///
/// * `finest_resolution` - The finest timestamp resolution.
/// * `divisor` - The factor by which to divide the resolution to get the interval duration.
///
/// # Returns
///
/// A `Duration` representing the interval duration.
pub fn calculate_interval_duration(
    finest_resolution: TimestampResolution,
    divisor: u32,
) -> Duration {
    // Convert finest_resolution to std::time::Duration
    let duration: Duration = finest_resolution.to_duration();

    // Divide the duration by the divisor, ensuring it's at least 1 nanosecond
    let divided = duration
        .checked_div(divisor)
        .unwrap_or(Duration::from_nanos(1));
    if divided.as_nanos() == 0 {
        Duration::from_nanos(1)
    } else {
        divided
    }
}

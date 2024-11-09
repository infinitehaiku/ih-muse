use std::cmp::Ordering;
use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

#[derive(PartialEq, Eq, Debug, Clone, Copy, Hash, Default, Deserialize, Serialize)]
pub enum TimestampResolution {
    Years,
    Months,
    Weeks,
    Days,
    Hours,
    Minutes,
    #[default]
    Seconds,
    Milliseconds,
    Microseconds,
}

impl FromStr for TimestampResolution {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        TimestampResolution::iter_ordered()
            .find(|&res| res.as_str().eq_ignore_ascii_case(s))
            .ok_or_else(|| format!("Invalid resolution: {}", s))
    }
}

impl fmt::Display for TimestampResolution {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

impl TimestampResolution {
    // Method to return an iterator over TimestampResolution from finest to coarsest
    pub fn iter_ordered() -> impl Iterator<Item = TimestampResolution> {
        [
            TimestampResolution::Microseconds,
            TimestampResolution::Milliseconds,
            TimestampResolution::Seconds,
            TimestampResolution::Minutes,
            TimestampResolution::Hours,
            TimestampResolution::Days,
            TimestampResolution::Weeks,
            TimestampResolution::Months,
            TimestampResolution::Years,
        ]
        .iter()
        .copied()
    }

    /// Method to get the string representation of the enum variant
    pub fn as_str(&self) -> &'static str {
        match *self {
            TimestampResolution::Years => "Years",
            TimestampResolution::Months => "Months",
            TimestampResolution::Weeks => "Weeks",
            TimestampResolution::Days => "Days",
            TimestampResolution::Hours => "Hours",
            TimestampResolution::Minutes => "Minutes",
            TimestampResolution::Seconds => "Seconds",
            TimestampResolution::Milliseconds => "Milliseconds",
            TimestampResolution::Microseconds => "Microseconds",
        }
    }
}

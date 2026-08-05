//! Fire Levels 1–5 and Five-Alarm escalation rules.

use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;

use crate::error::{Result, TifError};

/// Discrete aggressiveness levels for containment pressure.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[repr(u8)]
pub enum FireLevel {
    /// Light brevity and reuse guidance.
    Ember = 1,
    /// Stronger YAGNI pressure.
    Smolder = 2,
    /// Default guarded mode.
    Containment = 3,
    /// Aggressive reduction.
    Critical = 4,
    /// Maximum restraint; escalation only after containment failure.
    FiveAlarm = 5,
}

impl FireLevel {
    pub const MIN: u8 = 1;
    pub const MAX: u8 = 5;

    /// All levels in order.
    pub fn all() -> [FireLevel; 5] {
        [
            FireLevel::Ember,
            FireLevel::Smolder,
            FireLevel::Containment,
            FireLevel::Critical,
            FireLevel::FiveAlarm,
        ]
    }

    pub fn as_u8(self) -> u8 {
        self as u8
    }

    pub fn name(self) -> &'static str {
        match self {
            FireLevel::Ember => "Ember",
            FireLevel::Smolder => "Smolder",
            FireLevel::Containment => "Containment",
            FireLevel::Critical => "Critical",
            FireLevel::FiveAlarm => "Five-Alarm",
        }
    }

    /// Five-Alarm cannot be chosen for an initial implementation.
    pub fn is_initial_selectable(self) -> bool {
        !matches!(self, FireLevel::FiveAlarm)
    }

    /// Validate a user/CLI request for an *initial* fire level.
    pub fn parse_initial(value: u8) -> Result<FireLevel> {
        let level = FireLevel::try_from(value)?;
        if !level.is_initial_selectable() {
            return Err(TifError::FiveAlarmInitialForbidden);
        }
        Ok(level)
    }

    /// Parse any level including Five-Alarm (for escalation paths).
    pub fn parse_any(value: u8) -> Result<FireLevel> {
        FireLevel::try_from(value)
    }

    /// Whether Five-Alarm may activate given a *current* containment failure.
    ///
    /// Historical risk alone is insufficient; at least one current failure is required.
    pub fn may_escalate_to_five_alarm(current_containment_failure: bool) -> bool {
        current_containment_failure
    }

    /// Escalate to Five-Alarm only after a concrete containment failure.
    pub fn escalate_to_five_alarm(current_containment_failure: bool) -> Result<FireLevel> {
        if Self::may_escalate_to_five_alarm(current_containment_failure) {
            Ok(FireLevel::FiveAlarm)
        } else {
            Err(TifError::FiveAlarmInitialForbidden)
        }
    }

    /// Numeric multiplier used by policy/scoring (higher = stricter).
    pub fn pressure_multiplier(self) -> f64 {
        match self {
            FireLevel::Ember => 0.6,
            FireLevel::Smolder => 0.8,
            FireLevel::Containment => 1.0,
            FireLevel::Critical => 1.35,
            FireLevel::FiveAlarm => 1.75,
        }
    }
}

impl TryFrom<u8> for FireLevel {
    type Error = TifError;

    fn try_from(value: u8) -> Result<Self> {
        match value {
            1 => Ok(FireLevel::Ember),
            2 => Ok(FireLevel::Smolder),
            3 => Ok(FireLevel::Containment),
            4 => Ok(FireLevel::Critical),
            5 => Ok(FireLevel::FiveAlarm),
            other => Err(TifError::InvalidFireLevel(format!(
                "{other} (valid range {}–{})",
                FireLevel::MIN,
                FireLevel::MAX
            ))),
        }
    }
}

impl FromStr for FireLevel {
    type Err = TifError;

    fn from_str(s: &str) -> Result<Self> {
        let trimmed = s.trim();
        if let Ok(n) = trimmed.parse::<u8>() {
            return FireLevel::try_from(n);
        }
        match trimmed.to_ascii_lowercase().as_str() {
            "ember" | "1" => Ok(FireLevel::Ember),
            "smolder" | "2" => Ok(FireLevel::Smolder),
            "containment" | "3" => Ok(FireLevel::Containment),
            "critical" | "4" => Ok(FireLevel::Critical),
            "five-alarm" | "five_alarm" | "fivealarm" | "5" => Ok(FireLevel::FiveAlarm),
            other => Err(TifError::InvalidFireLevel(other.to_string())),
        }
    }
}

impl fmt::Display for FireLevel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} ({})", self.as_u8(), self.name())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn initial_selection_forbids_five_alarm() {
        assert!(FireLevel::parse_initial(5).is_err());
        assert!(FireLevel::parse_initial(3).is_ok());
        assert!(!FireLevel::FiveAlarm.is_initial_selectable());
    }

    #[test]
    fn five_alarm_requires_current_failure() {
        assert!(FireLevel::escalate_to_five_alarm(false).is_err());
        assert_eq!(
            FireLevel::escalate_to_five_alarm(true).unwrap(),
            FireLevel::FiveAlarm
        );
    }

    #[test]
    fn parse_names() {
        assert_eq!(
            FireLevel::from_str("critical").unwrap(),
            FireLevel::Critical
        );
        assert_eq!(FireLevel::from_str("4").unwrap(), FireLevel::Critical);
    }
}

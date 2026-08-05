//! Reviewer pool and adaptive selector (user-authorized models only).

use serde::{Deserialize, Serialize};

use crate::config::ReviewerConfig;
use crate::error::{Result, TifError};
use crate::task::TaskCategory;

/// Selection of a reviewer for Firebreak.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SelectedReviewer {
    pub id: String,
    pub provider: String,
    pub model: String,
    pub allow_source_egress: bool,
    pub max_firebreak_attempts: u32,
}

/// Local ranking stats (adaptation input).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ReviewerStats {
    pub reviewer_id: String,
    pub attempts: u32,
    pub successes: u32,
    pub avg_reduction_score: f64,
}

/// Selects only from the user-authorized reviewer pool.
#[derive(Debug, Clone)]
pub struct ReviewerSelector {
    pool: Vec<ReviewerConfig>,
    stats: Vec<ReviewerStats>,
}

impl ReviewerSelector {
    pub fn new(pool: Vec<ReviewerConfig>) -> Self {
        Self {
            pool,
            stats: Vec::new(),
        }
    }

    pub fn with_stats(mut self, stats: Vec<ReviewerStats>) -> Self {
        self.stats = stats;
        self
    }

    pub fn pool(&self) -> &[ReviewerConfig] {
        &self.pool
    }

    /// Select best authorized reviewer for the task. Never invents unauthorized models.
    pub fn select(&self, category: TaskCategory) -> Result<SelectedReviewer> {
        if self.pool.is_empty() {
            return Err(TifError::UnauthorizedReviewer(
                "no reviewers authorized in local configuration".into(),
            ));
        }

        let eligible: Vec<&ReviewerConfig> = self
            .pool
            .iter()
            .filter(|r| {
                r.eligible_task_types.is_empty()
                    || r.eligible_task_types
                        .iter()
                        .any(|t| t == category.as_str() || t == "*")
            })
            .collect();

        if eligible.is_empty() {
            return Err(TifError::UnauthorizedReviewer(format!(
                "no authorized reviewer eligible for task {}",
                category.as_str()
            )));
        }

        // Rank by local success rate, then priority.
        let mut ranked = eligible;
        ranked.sort_by(|a, b| {
            let sa = self.success_rate(&a.id);
            let sb = self.success_rate(&b.id);
            sb.partial_cmp(&sa)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| b.priority.cmp(&a.priority))
                .then_with(|| a.id.cmp(&b.id))
        });

        let best = ranked[0];
        Ok(SelectedReviewer {
            id: best.id.clone(),
            provider: best.provider.clone(),
            model: best.model.clone(),
            allow_source_egress: best.allow_source_egress,
            max_firebreak_attempts: best.max_firebreak_attempts,
        })
    }

    fn success_rate(&self, id: &str) -> f64 {
        self.stats
            .iter()
            .find(|s| s.reviewer_id == id)
            .map(|s| {
                if s.attempts == 0 {
                    0.0
                } else {
                    s.successes as f64 / s.attempts as f64
                }
            })
            .unwrap_or(0.0)
    }

    /// Record a local outcome for adaptation.
    pub fn record_outcome(&mut self, reviewer_id: &str, success: bool, reduction: f64) {
        if let Some(s) = self.stats.iter_mut().find(|s| s.reviewer_id == reviewer_id) {
            s.attempts += 1;
            if success {
                s.successes += 1;
            }
            s.avg_reduction_score =
                (s.avg_reduction_score * (s.attempts as f64 - 1.0) + reduction) / s.attempts as f64;
        } else {
            self.stats.push(ReviewerStats {
                reviewer_id: reviewer_id.to_string(),
                attempts: 1,
                successes: u32::from(success),
                avg_reduction_score: reduction,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rev(id: &str, priority: i32) -> ReviewerConfig {
        ReviewerConfig::mock(id, priority)
    }

    #[test]
    fn empty_pool_errors() {
        let s = ReviewerSelector::new(vec![]);
        assert!(s.select(TaskCategory::BugFix).is_err());
    }

    #[test]
    fn selects_highest_priority_without_stats() {
        let s = ReviewerSelector::new(vec![rev("a", 1), rev("b", 10)]);
        let sel = s.select(TaskCategory::BugFix).unwrap();
        assert_eq!(sel.id, "b");
    }

    #[test]
    fn respects_task_eligibility() {
        let mut r = rev("sec", 100);
        r.eligible_task_types = vec!["security_remediation".into()];
        let s = ReviewerSelector::new(vec![r, rev("gen", 1)]);
        let sel = s.select(TaskCategory::BugFix).unwrap();
        assert_eq!(sel.id, "gen");
    }
}

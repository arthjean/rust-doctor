use std::collections::BTreeMap;

use super::density::{DensityScope, cause_weight};

const MICRO: f64 = 1_000_000.0;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct ChargedSites {
    total_micro: u64,
    owned_micro: u64,
}

impl ChargedSites {
    #[cfg(test)]
    pub(super) const fn whole(sites: u64, owned: u64) -> Self {
        Self {
            total_micro: sites.saturating_mul(1_000_000),
            owned_micro: owned.saturating_mul(1_000_000),
        }
    }

    fn of(total: f64, owned: f64) -> Self {
        let micro = |sites: f64| {
            let scaled = (sites * MICRO).round();
            if scaled.is_finite() && scaled > 0.0 { scaled as u64 } else { 0 }
        };
        let total_micro = micro(total);
        Self {
            total_micro,
            owned_micro: micro(owned).min(total_micro),
        }
    }

    pub(crate) fn total(self) -> f64 {
        self.total_micro as f64 / MICRO
    }

    pub(super) fn owned(self) -> f64 {
        self.owned_micro as f64 / MICRO
    }

    pub(super) fn unowned(self) -> f64 {
        self.total_micro.saturating_sub(self.owned_micro) as f64 / MICRO
    }
}

#[derive(Debug, Clone, Copy, Default)]
struct Cause {
    findings: u64,
    weight: u64,
    owned: u64,
}

impl Cause {
    fn add(&mut self, weight: u64, owned: bool) {
        self.findings = self.findings.saturating_add(1);
        self.weight = self.weight.saturating_add(weight);
        if owned {
            self.owned = self.owned.saturating_add(weight);
        }
    }

    fn scaled(self, factor: f64) -> (f64, f64) {
        (self.weight as f64 * factor, self.owned as f64 * factor)
    }

    fn per_finding(self) -> f64 {
        if self.findings == 0 { 0.0 } else { 1.0 / self.findings as f64 }
    }
}

#[derive(Debug, Clone, Default)]
pub(super) struct CauseTally {
    declared: BTreeMap<String, Cause>,
    singles: Cause,
}

impl CauseTally {
    pub(super) fn add(&mut self, cause: Option<&str>, weight: u64, owned: bool) {
        if weight == 0 {
            return;
        }
        match cause {
            Some(cause) => self
                .declared
                .entry(cause.to_owned())
                .or_default()
                .add(weight, owned),
            None => self.singles.add(weight, owned),
        }
    }

    fn causes(&self) -> u64 {
        (self.declared.len() as u64).saturating_add(self.singles.findings)
    }

    pub(super) fn charged(&self, scope: DensityScope) -> ChargedSites {
        let (singles, singles_owned) = self.singles.scaled(1.0);
        let (total, owned) = match scope {
            DensityScope::PerKiloline => self.declared.values().fold(
                (singles, singles_owned),
                |(total, owned), cause| {
                    let (weight, held) =
                        cause.scaled(cause_weight(cause.findings as f64) * cause.per_finding());
                    (total + weight, owned + held)
                },
            ),
            DensityScope::Workspace => {
                let (once, once_owned) = self.declared.values().fold(
                    (singles, singles_owned),
                    |(total, owned), cause| {
                        let (weight, held) = cause.scaled(cause.per_finding());
                        (total + weight, owned + held)
                    },
                );
                let causes = self.causes() as f64;
                let saturation = if causes > 0.0 {
                    cause_weight(causes) / causes
                } else {
                    0.0
                };
                (once * saturation, once_owned * saturation)
            }
        };
        ChargedSites::of(total, owned)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tally(declared: &[(&str, u64)], singles: u64) -> CauseTally {
        let mut tally = CauseTally::default();
        for (cause, findings) in declared {
            for _ in 0..*findings {
                tally.add(Some(cause), 1, false);
            }
        }
        for _ in 0..singles {
            tally.add(None, 1, false);
        }
        tally
    }

    #[test]
    fn three_hundred_and_three_sites_of_one_cause_weigh_one_plus_their_logarithm() {
        let charged = tally(&[("UiColors (292 byte)", 303)], 7).charged(DensityScope::PerKiloline);
        assert!((charged.total() - (1.0 + 303f64.ln() + 7.0)).abs() < 0.001, "{}", charged.total());
    }

    #[test]
    fn a_rule_with_no_declared_cause_is_charged_as_under_core_v6_per_kiloline() {
        let mut tally = CauseTally::default();
        for weight in [1, 2, 1, 0, 2] {
            tally.add(None, weight, weight == 2);
        }
        let charged = tally.charged(DensityScope::PerKiloline);
        assert_eq!(charged, ChargedSites::whole(6, 4));
    }

    #[test]
    fn a_workspace_rule_counts_each_cause_once_on_the_same_curve() {
        assert_eq!(tally(&[], 1).charged(DensityScope::Workspace), ChargedSites::whole(1, 0));
        assert_eq!(tally(&[("API_KEY", 3)], 0).charged(DensityScope::Workspace), ChargedSites::whole(1, 0));
        let three = tally(&[], 3).charged(DensityScope::Workspace);
        assert!((three.total() - (1.0 + 3f64.ln())).abs() < 1e-6, "{}", three.total());
        let mixed = tally(&[("API_KEY", 3), ("OTHER", 2)], 1).charged(DensityScope::Workspace);
        assert!((mixed.total() - (1.0 + 3f64.ln())).abs() < 1e-6, "{}", mixed.total());
        assert_eq!(CauseTally::default().charged(DensityScope::Workspace), ChargedSites::default());
    }

    #[test]
    fn an_owned_share_follows_its_cause_through_the_curve() {
        let mut tally = CauseTally::default();
        tally.add(None, 1, true);
        tally.add(None, 1, false);
        let charged = tally.charged(DensityScope::Workspace);
        let expected = (1.0 + 2f64.ln()) / 2.0;
        assert!((charged.owned() - expected).abs() < 1e-6);
        assert!((charged.unowned() - expected).abs() < 1e-6);
    }
}

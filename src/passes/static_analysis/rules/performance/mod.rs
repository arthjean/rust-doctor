mod allocation;
mod clone;
mod collect_iterate;
mod large_enum;
mod string_literal;

pub use allocation::UnnecessaryAllocation;
pub use clone::ExcessiveClone;
pub use collect_iterate::CollectThenIterate;
pub use large_enum::LargeEnumVariant;
pub use string_literal::StringFromLiteral;

use crate::rules::CustomRule;

/// Returns all performance rules.
pub fn all_rules() -> Vec<Box<dyn CustomRule>> {
    vec![
        Box::new(ExcessiveClone::default()),
        Box::new(StringFromLiteral),
        Box::new(CollectThenIterate),
        Box::new(LargeEnumVariant),
        Box::new(UnnecessaryAllocation),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_all_rules_returns_5() {
        assert_eq!(all_rules().len(), 5);
    }
}

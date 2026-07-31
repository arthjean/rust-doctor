#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Activation {
    Warning,
}

impl Activation {
    pub(crate) const fn flag(self) -> &'static str {
        match self {
            Self::Warning => "-W",
        }
    }

    #[cfg(test)]
    const fn level(self) -> &'static str {
        match self {
            Self::Warning => "warning",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Rule {
    pub(crate) code: &'static str,
    pub(crate) category: &'static str,
    pub(crate) activation: Activation,
    pub(crate) help: &'static str,
}

pub(crate) const RULES: [Rule; 3] = [
    Rule {
        code: "clippy::dbg_macro",
        category: "maintainability",
        activation: Activation::Warning,
        help: "Remove dbg! or replace it with intentional logging.",
    },
    Rule {
        code: "clippy::todo",
        category: "correctness",
        activation: Activation::Warning,
        help: "Replace todo! with the intended implementation or remove the reachable placeholder.",
    },
    Rule {
        code: "clippy::unimplemented",
        category: "correctness",
        activation: Activation::Warning,
        help: "Implement this code path or remove the reachable placeholder.",
    },
];

pub(crate) fn find(code: &str) -> Option<&'static Rule> {
    RULES
        .binary_search_by_key(&code, |rule| rule.code)
        .ok()
        .map(|index| &RULES[index])
}

#[cfg(test)]
#[derive(Debug, PartialEq, Eq)]
enum RegistryError {
    Count,
    DuplicateCode,
    Unsorted,
    EmptyMetadata,
}

#[cfg(test)]
fn validate(rules: &[Rule]) -> Result<(), RegistryError> {
    if rules.windows(2).any(|pair| pair[0].code == pair[1].code) {
        return Err(RegistryError::DuplicateCode);
    }
    if rules.len() != RULES.len() {
        return Err(RegistryError::Count);
    }
    if rules.windows(2).any(|pair| pair[0].code > pair[1].code) {
        return Err(RegistryError::Unsorted);
    }
    if rules.iter().any(|rule| {
        rule.code.is_empty()
            || rule.category.is_empty()
            || rule.activation.level().is_empty()
            || rule.help.is_empty()
    }) {
        return Err(RegistryError::EmptyMetadata);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_is_the_exact_normative_inventory() {
        validate(&RULES).unwrap();
        assert_eq!(
            RULES,
            [
                Rule {
                    code: "clippy::dbg_macro",
                    category: "maintainability",
                    activation: Activation::Warning,
                    help: "Remove dbg! or replace it with intentional logging.",
                },
                Rule {
                    code: "clippy::todo",
                    category: "correctness",
                    activation: Activation::Warning,
                    help: "Replace todo! with the intended implementation or remove the reachable placeholder.",
                },
                Rule {
                    code: "clippy::unimplemented",
                    category: "correctness",
                    activation: Activation::Warning,
                    help: "Implement this code path or remove the reachable placeholder.",
                },
            ]
        );
        assert!(
            RULES
                .iter()
                .all(|rule| rule.activation.level() == "warning")
        );
        assert!(RULES.iter().all(|rule| {
            !std::path::Path::new(rule.help).is_absolute()
                && !rule.category.chars().any(char::is_control)
                && !rule.help.chars().any(char::is_control)
        }));
    }

    #[test]
    fn exact_lookup_does_not_match_similar_codes() {
        assert_eq!(
            find("clippy::todo").map(|rule| rule.category),
            Some("correctness")
        );
        assert!(find("clippy::todo_suffix").is_none());
        assert!(find("todo").is_none());
    }

    #[test]
    fn duplicate_registry_code_is_rejected_without_running_a_process() {
        let duplicate = [RULES[0], RULES[0], RULES[2]];

        assert_eq!(validate(&duplicate), Err(RegistryError::DuplicateCode));
    }
}

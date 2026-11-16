#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CauseKey {
    TypeAndSize,
    StepVariable,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct CauseSite<'a> {
    pub(crate) message: &'a str,
    pub(crate) spanned: Option<&'a str>,
}

impl CauseKey {
    pub(crate) const fn reads_the_span(self) -> bool {
        matches!(self, Self::TypeAndSize)
    }

    pub(crate) fn of(self, site: CauseSite<'_>) -> Option<String> {
        match self {
            Self::TypeAndSize => type_and_size(site),
            Self::StepVariable => step_variable(site.message),
        }
    }
}

fn type_and_size(site: CauseSite<'_>) -> Option<String> {
    let spanned = site.spanned?.trim();
    if spanned.is_empty() || spanned.contains('\n') {
        return None;
    }
    let segment = last_path_segment(spanned)?;
    let size = byte_size(site.message)?;
    Some(format!("{segment} ({size} byte)"))
}

fn last_path_segment(path: &str) -> Option<&str> {
    let mut depth = 0usize;
    let mut start = 0usize;
    let mut previous = None;
    for (index, character) in path.char_indices() {
        match character {
            '<' | '(' | '[' => depth += 1,
            '>' | ')' | ']' => depth = depth.saturating_sub(1),
            ':' if depth == 0 && previous == Some(':') => start = index + 1,
            _ => {}
        }
        previous = Some(character);
    }
    path.get(start..).map(str::trim).filter(|segment| !segment.is_empty())
}

fn byte_size(message: &str) -> Option<&str> {
    let (before, _) = message.split_once(" byte)")?;
    let (_, size) = before.rsplit_once('(')?;
    (!size.is_empty() && size.bytes().all(|byte| byte.is_ascii_digit())).then_some(size)
}

fn step_variable(message: &str) -> Option<String> {
    let variable = message
        .split_once("holds the secret ")
        .and_then(|(_, rest)| rest.split_once(','))
        .map(|(variable, _)| variable)
        .or_else(|| {
            message
                .split_once("compiles with `")
                .and_then(|(_, rest)| rest.split_once("` in scope"))
                .map(|(variable, _)| variable)
        })?;
    let variable = variable.trim();
    (!variable.is_empty()
        && variable
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_'))
    .then(|| variable.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn site<'a>(message: &'a str, spanned: Option<&'a str>) -> CauseSite<'a> {
        CauseSite { message, spanned }
    }

    const BY_VALUE: &str = "this argument (292 byte) is passed by value, but might be more efficient if passed by reference (limit: 256 byte)";

    #[test]
    fn a_by_value_type_is_named_by_its_last_segment_and_its_size() {
        for spanned in ["UiColors", "crate::theme::UiColors", " theme::UiColors "] {
            assert_eq!(
                CauseKey::TypeAndSize.of(site(BY_VALUE, Some(spanned))).as_deref(),
                Some("UiColors (292 byte)"),
                "{spanned}"
            );
        }
        assert_eq!(
            CauseKey::TypeAndSize
                .of(site(BY_VALUE, Some("Option<crate::theme::UiColors>")))
                .as_deref(),
            Some("Option<crate::theme::UiColors> (292 byte)")
        );
        assert_eq!(
            CauseKey::TypeAndSize
                .of(site(BY_VALUE, Some("std::sync::Arc<crate::A>")))
                .as_deref(),
            Some("Arc<crate::A> (292 byte)")
        );
    }

    #[test]
    fn a_span_that_cannot_be_read_declares_no_cause() {
        assert_eq!(CauseKey::TypeAndSize.of(site(BY_VALUE, None)), None);
        assert_eq!(CauseKey::TypeAndSize.of(site(BY_VALUE, Some("  "))), None);
        assert_eq!(CauseKey::TypeAndSize.of(site(BY_VALUE, Some("A\nB"))), None);
        assert_eq!(
            CauseKey::TypeAndSize.of(site("this argument is passed by value", Some("A"))),
            None
        );
    }

    #[test]
    fn a_secret_in_a_build_step_is_caused_by_its_variable() {
        assert_eq!(
            CauseKey::StepVariable
                .of(site(
                    "cargo build compiles the workspace while the step env holds the secret API_KEY, which any build script can read",
                    None
                ))
                .as_deref(),
            Some("API_KEY")
        );
        assert_eq!(
            CauseKey::StepVariable
                .of(site(
                    "`cargo build` compiles with `API_KEY` in scope, and `option_env!` at src/lib.rs:3 embeds it in the binary: whatever it holds ships to every user.",
                    None
                ))
                .as_deref(),
            Some("API_KEY")
        );
        assert_eq!(CauseKey::StepVariable.of(site("no variable named", None)), None);
        assert!(!CauseKey::StepVariable.reads_the_span());
        assert!(CauseKey::TypeAndSize.reads_the_span());
    }
}

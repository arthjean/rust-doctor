//! Ownership-aware rust-doctor skill generation.

/// The skill template bundled at compile time.
const SKILL_TEMPLATE: &str = include_str!("templates/skill.md");

/// Marker placed after YAML front matter so skill parsers still see `---` as
/// the first line.
const SKILL_MARKER: &str = "<!-- rust-doctor-managed-skill:v1 -->";

#[derive(Debug, Eq, PartialEq)]
pub(super) enum SkillEdit {
    Unchanged,
    Write(Vec<u8>),
    Delete,
}

pub(super) fn install(existing: Option<&[u8]>) -> Result<SkillEdit, String> {
    let desired = managed_content().into_bytes();
    match existing {
        None => Ok(SkillEdit::Write(desired)),
        Some(content) if content == desired => Ok(SkillEdit::Unchanged),
        Some(content) if is_owned(content) => Ok(SkillEdit::Write(desired)),
        Some(_) => Err(
            "the rust-doctor skill path contains an unmanaged file; move it or remove it explicitly"
                .to_owned(),
        ),
    }
}

pub(super) fn uninstall(existing: Option<&[u8]>) -> Result<SkillEdit, String> {
    match existing {
        None => Ok(SkillEdit::Unchanged),
        Some(content) if is_owned(content) => Ok(SkillEdit::Delete),
        Some(_) => Err("refusing to remove an unmanaged rust-doctor skill file".to_owned()),
    }
}

fn is_owned(content: &[u8]) -> bool {
    content == SKILL_TEMPLATE.as_bytes()
        || std::str::from_utf8(content).is_ok_and(has_valid_managed_marker)
}

fn has_valid_managed_marker(content: &str) -> bool {
    let Some(front_matter_end) = content.find("\n---\n") else {
        return false;
    };
    let insertion = front_matter_end + "\n---\n".len();
    let front_matter = &content[..front_matter_end];
    front_matter.lines().any(|line| line == "name: rust-doctor")
        && content[insertion..].starts_with(SKILL_MARKER)
        && content[insertion + SKILL_MARKER.len()..].starts_with('\n')
        && content.matches(SKILL_MARKER).count() == 1
}

fn managed_content() -> String {
    let Some(front_matter_end) = SKILL_TEMPLATE.find("\n---\n") else {
        let mut output = SKILL_TEMPLATE.to_owned();
        output.push('\n');
        output.push_str(SKILL_MARKER);
        output.push('\n');
        return output;
    };
    let insertion = front_matter_end + "\n---\n".len();
    let mut output = String::with_capacity(SKILL_TEMPLATE.len() + SKILL_MARKER.len() + 1);
    output.push_str(&SKILL_TEMPLATE[..insertion]);
    output.push_str(SKILL_MARKER);
    output.push('\n');
    output.push_str(&SKILL_TEMPLATE[insertion..]);
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn marker_follows_front_matter() {
        let content = managed_content();
        assert!(content.starts_with("---\n"));
        let closing = content
            .find("\n---\n")
            .expect("front matter closing marker");
        assert!(content[closing..].contains(SKILL_MARKER));
    }

    #[test]
    fn managed_install_is_idempotent() {
        let desired = managed_content();
        assert_eq!(
            install(Some(desired.as_bytes())).expect("managed install"),
            SkillEdit::Unchanged
        );
    }

    #[test]
    fn legacy_bundled_skill_can_be_upgraded() {
        assert!(matches!(
            install(Some(SKILL_TEMPLATE.as_bytes())).expect("legacy upgrade"),
            SkillEdit::Write(_)
        ));
    }

    #[test]
    fn unmanaged_skill_is_never_overwritten_or_removed() {
        let custom = b"---\nname: rust-doctor\n---\ncustom instructions\n";
        assert!(install(Some(custom)).is_err());
        assert!(uninstall(Some(custom)).is_err());
    }

    #[test]
    fn marker_mentioned_outside_the_managed_position_does_not_claim_ownership() {
        let custom =
            format!("---\nname: rust-doctor\n---\ncustom instructions mentioning {SKILL_MARKER}\n");
        assert!(install(Some(custom.as_bytes())).is_err());
        assert!(uninstall(Some(custom.as_bytes())).is_err());
    }

    #[test]
    fn uninstall_recognizes_only_owned_content() {
        let desired = managed_content();
        assert_eq!(
            uninstall(Some(desired.as_bytes())).expect("managed uninstall"),
            SkillEdit::Delete
        );
        assert_eq!(
            uninstall(None).expect("missing skill"),
            SkillEdit::Unchanged
        );
    }
}

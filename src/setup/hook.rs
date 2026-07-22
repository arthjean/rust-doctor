//! Namespace-scoped staged pre-commit hook generation.

use super::BlockingLevel;

const START_MARKER: &str = "# >>> rust-doctor managed pre-commit:v1 >>>";
const END_MARKER: &str = "# <<< rust-doctor managed pre-commit:v1 <<<";
const CREATED_MARKER: &str = "# rust-doctor-created-hook:v1";

#[derive(Debug, Eq, PartialEq)]
pub(super) enum HookEdit {
    Unchanged,
    Write(Vec<u8>),
    Delete,
}

pub(super) fn install(
    existing: Option<&[u8]>,
    blocking: BlockingLevel,
) -> Result<HookEdit, String> {
    let block = managed_block(blocking);
    let Some(existing) = existing else {
        return Ok(HookEdit::Write(new_hook(&block).into_bytes()));
    };
    if existing.is_empty() {
        return Ok(HookEdit::Write(new_hook(&block).into_bytes()));
    }
    let existing = std::str::from_utf8(existing)
        .map_err(|_| "pre-commit hook is not valid UTF-8".to_owned())?;
    if !has_supported_shell_shebang(existing) {
        return Err(
            "pre-commit hook uses an unsupported interpreter; keep the existing hook and invoke rust-doctor from that hook explicitly"
                .to_owned(),
        );
    }
    let markers = marker_range(existing)?;

    let Some((start, end)) = markers else {
        let mut output = String::with_capacity(existing.len() + block.len() + 1);
        output.push_str(existing);
        if !existing.is_empty() {
            // Always insert one delimiter byte. Uninstall removes this exact
            // delimiter and can therefore restore unrelated content byte-for-byte.
            output.push('\n');
        }
        output.push_str(&block);
        return Ok(HookEdit::Write(output.into_bytes()));
    };

    if &existing[start..end] == block.as_str() {
        return Ok(HookEdit::Unchanged);
    }
    let mut output = String::with_capacity(existing.len() - (end - start) + block.len());
    output.push_str(&existing[..start]);
    output.push_str(&block);
    output.push_str(&existing[end..]);
    Ok(HookEdit::Write(output.into_bytes()))
}

fn has_supported_shell_shebang(content: &str) -> bool {
    let Some(shebang) = content
        .lines()
        .next()
        .and_then(|line| line.strip_prefix("#!"))
    else {
        return false;
    };
    shebang.split_whitespace().any(|part| {
        part.rsplit(['/', '\\'])
            .next()
            .is_some_and(|name| matches!(name, "sh" | "bash" | "dash" | "ksh" | "zsh"))
    })
}

pub(super) fn uninstall(existing: Option<&[u8]>) -> Result<HookEdit, String> {
    let Some(existing) = existing else {
        return Ok(HookEdit::Unchanged);
    };
    let existing = std::str::from_utf8(existing)
        .map_err(|_| "pre-commit hook is not valid UTF-8".to_owned())?;

    if is_fully_managed(existing)? {
        return Ok(HookEdit::Delete);
    }
    let Some((mut start, end)) = marker_range(existing)? else {
        return Ok(HookEdit::Unchanged);
    };

    let created_prefix = format!("{CREATED_MARKER}\n");
    if existing[..start].ends_with(&created_prefix) {
        start -= created_prefix.len();
    } else if start > 0 && existing.as_bytes()[start - 1] == b'\n' {
        start -= 1;
    }
    let mut output = String::with_capacity(existing.len() - (end - start));
    output.push_str(&existing[..start]);
    output.push_str(&existing[end..]);
    Ok(HookEdit::Write(output.into_bytes()))
}

fn managed_block(blocking: BlockingLevel) -> String {
    format!(
        "{START_MARKER}\nrust-doctor . --staged --blocking {} || exit $?\n{END_MARKER}\n",
        blocking.as_str()
    )
}

fn new_hook(block: &str) -> String {
    let mut output =
        String::with_capacity("#!/bin/sh\n".len() + CREATED_MARKER.len() + 1 + block.len());
    output.push_str("#!/bin/sh\n");
    output.push_str(CREATED_MARKER);
    output.push('\n');
    output.push_str(block);
    output
}

fn is_fully_managed(content: &str) -> Result<bool, String> {
    let Some((start, end)) = marker_range(content)? else {
        return Ok(false);
    };
    let prefix = format!("#!/bin/sh\n{CREATED_MARKER}\n");
    Ok(start == prefix.len() && content.starts_with(&prefix) && end == content.len())
}

/// Return the byte range containing both marker lines and their managed body.
fn marker_range(content: &str) -> Result<Option<(usize, usize)>, String> {
    let starts: Vec<_> = content.match_indices(START_MARKER).collect();
    let ends: Vec<_> = content.match_indices(END_MARKER).collect();
    match (starts.as_slice(), ends.as_slice()) {
        ([], []) => Ok(None),
        ([(start, _)], [(end, _)]) if start < end => {
            let mut range_end = *end + END_MARKER.len();
            if range_end < content.len() && content.as_bytes()[range_end] == b'\n' {
                range_end += 1;
            }
            Ok(Some((*start, range_end)))
        }
        _ => Err(format!(
            "conflicting rust-doctor hook namespace: expected one `{START_MARKER}` and one `{END_MARKER}`"
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_hook_scans_staged_files_with_blocking_level() {
        let HookEdit::Write(content) = install(None, BlockingLevel::Warning).expect("hook install")
        else {
            panic!("new hook should be written");
        };
        let content = String::from_utf8(content).expect("UTF-8 hook");
        assert!(content.starts_with("#!/bin/sh\n"));
        assert!(content.contains("rust-doctor . --staged --blocking warning || exit $?"));
        assert!(content.contains(START_MARKER));
        assert!(content.contains(END_MARKER));
    }

    #[test]
    fn install_then_uninstall_restores_unrelated_hook_exactly() {
        let original = "#!/bin/sh\necho user-hook";
        let HookEdit::Write(installed) =
            install(Some(original.as_bytes()), BlockingLevel::Error).expect("hook install")
        else {
            panic!("hook should change");
        };
        let HookEdit::Write(restored) = uninstall(Some(&installed)).expect("hook uninstall") else {
            panic!("mixed hook should be rewritten");
        };
        assert_eq!(restored, original.as_bytes());
    }

    #[test]
    fn repeated_install_is_idempotent() {
        let HookEdit::Write(first) = install(None, BlockingLevel::None).expect("first install")
        else {
            panic!("new hook should be written");
        };
        assert_eq!(
            install(Some(&first), BlockingLevel::None).expect("second install"),
            HookEdit::Unchanged
        );
    }

    #[test]
    fn empty_existing_hook_gets_a_shebang() {
        let HookEdit::Write(content) =
            install(Some(b""), BlockingLevel::Warning).expect("empty hook install")
        else {
            panic!("empty hook should be replaced");
        };
        assert!(content.starts_with(b"#!/bin/sh\n"));
    }

    #[test]
    fn uninstall_preserves_commands_added_after_a_created_hook() {
        let HookEdit::Write(mut content) =
            install(None, BlockingLevel::Warning).expect("hook install")
        else {
            panic!("new hook should be written");
        };
        content.extend_from_slice(b"echo user-command\n");

        let HookEdit::Write(restored) = uninstall(Some(&content)).expect("hook uninstall") else {
            panic!("mixed hook should be rewritten");
        };
        assert_eq!(restored, b"#!/bin/sh\necho user-command\n");
    }

    #[test]
    fn malformed_namespace_is_refused() {
        let content = format!("#!/bin/sh\n{START_MARKER}\necho broken\n");
        let error = install(Some(content.as_bytes()), BlockingLevel::Warning)
            .expect_err("unbalanced marker must fail");
        assert!(error.contains("conflicting"));
    }

    #[test]
    fn non_shell_hook_is_refused_without_rewriting_it() {
        let python = b"#!/usr/bin/env python3\nprint('user hook')\n";
        let error = install(Some(python), BlockingLevel::Warning)
            .expect_err("non-shell hook must be preserved");
        assert!(error.contains("unsupported interpreter"));
    }

    #[test]
    fn fully_managed_hook_is_deleted() {
        let HookEdit::Write(content) = install(None, BlockingLevel::Warning).expect("hook install")
        else {
            panic!("new hook should be written");
        };
        assert_eq!(
            uninstall(Some(&content)).expect("hook uninstall"),
            HookEdit::Delete
        );
    }
}

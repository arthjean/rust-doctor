//! The glob `rust-doctor.toml` writes a path with: `*`, `**` and `?`, relative
//! to the workspace root, and nothing else.
//!
//! A pattern names a file or a directory, and a directory covers everything
//! under it, so `vendor` and `vendor/**` say the same thing. What a pattern
//! cannot say is refused rather than read literally: a character class, a
//! brace, a negation, an absolute path, a `.` or `..` segment, and a `**` that
//! is not a whole segment. A glob read two ways is an exception nobody wrote.

/// One segment of a pattern.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Segment {
    /// `**`: any number of whole segments, none included.
    AnyDepth,
    /// Literal characters, `*` and `?`, matched inside one segment.
    Name(Vec<char>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PathGlob {
    pattern: String,
    segments: Vec<Segment>,
}

impl PathGlob {
    /// Reads a pattern, or refuses it. The refusal carries nothing: the
    /// caller names where the pattern was written, never what it said.
    pub(crate) fn parse(pattern: &str) -> Result<Self, ()> {
        let trimmed = pattern.strip_suffix('/').unwrap_or(pattern);
        if trimmed.is_empty()
            || trimmed.starts_with('/')
            || trimmed.chars().any(|character| {
                matches!(character, '[' | ']' | '{' | '}' | '!' | '\\') || character.is_control()
            })
        {
            return Err(());
        }
        let segments = trimmed
            .split('/')
            .map(|segment| match segment {
                "" | "." | ".." => Err(()),
                "**" => Ok(Segment::AnyDepth),
                _ if segment.contains("**") => Err(()),
                _ => Ok(Segment::Name(segment.chars().collect())),
            })
            .collect::<Result<_, _>>()?;
        Ok(Self {
            pattern: pattern.to_owned(),
            segments,
        })
    }

    /// The pattern as the configuration wrote it.
    pub(crate) fn as_str(&self) -> &str {
        &self.pattern
    }

    /// Does the pattern name this workspace-relative path, or a directory
    /// holding it?
    pub(crate) fn matches(&self, path: &str) -> bool {
        let segments: Vec<&str> = path.split('/').filter(|segment| !segment.is_empty()).collect();
        (1..=segments.len()).any(|depth| {
            segments
                .get(..depth)
                .is_some_and(|prefix| matches_segments(&self.segments, prefix))
        })
    }
}

fn matches_segments(pattern: &[Segment], path: &[&str]) -> bool {
    match pattern.split_first() {
        None => path.is_empty(),
        Some((Segment::AnyDepth, rest)) => {
            (0..=path.len()).any(|skip| path.get(skip..).is_some_and(|tail| matches_segments(rest, tail)))
        }
        Some((Segment::Name(name), rest)) => path.split_first().is_some_and(|(first, tail)| {
            matches_name(name, &first.chars().collect::<Vec<_>>()) && matches_segments(rest, tail)
        }),
    }
}

fn matches_name(pattern: &[char], name: &[char]) -> bool {
    match pattern.split_first() {
        None => name.is_empty(),
        Some(('*', rest)) => {
            (0..=name.len()).any(|skip| name.get(skip..).is_some_and(|tail| matches_name(rest, tail)))
        }
        Some(('?', rest)) => name
            .split_first()
            .is_some_and(|(_, tail)| matches_name(rest, tail)),
        Some((literal, rest)) => name
            .split_first()
            .is_some_and(|(first, tail)| first == literal && matches_name(rest, tail)),
    }
}

#[cfg(test)]
mod tests {
    use super::PathGlob;

    fn glob(pattern: &str) -> PathGlob {
        PathGlob::parse(pattern).unwrap()
    }

    #[test]
    fn a_directory_covers_everything_under_it() {
        for pattern in ["vendor", "vendor/", "vendor/**"] {
            assert!(glob(pattern).matches("vendor/a/b.rs"), "{pattern}");
            assert!(!glob(pattern).matches("src/vendor.rs"), "{pattern}");
        }
        assert!(!glob("vendor").matches("vendored/a.rs"));
    }

    #[test]
    fn star_stays_in_its_segment_and_double_star_crosses_them() {
        assert!(glob("src/*.rs").matches("src/lib.rs"));
        assert!(!glob("src/*.rs").matches("src/a/lib.rs"));
        assert!(glob("src/**/*.rs").matches("src/lib.rs"));
        assert!(glob("src/**/*.rs").matches("src/a/b/lib.rs"));
        assert!(glob("**/generated").matches("crates/x/src/generated/mod.rs"));
        assert!(glob("*_pb.rs").matches("x_pb.rs"));
        assert!(!glob("*_pb.rs").matches("src/x_pb.rs"));
        assert!(glob("**/*_pb.rs").matches("src/x_pb.rs"));
    }

    #[test]
    fn question_mark_is_one_character_of_one_segment() {
        assert!(glob("src/v?.rs").matches("src/v1.rs"));
        assert!(!glob("src/v?.rs").matches("src/v10.rs"));
        assert!(!glob("a?b").matches("a/b"));
    }

    #[test]
    fn what_a_pattern_cannot_say_is_refused() {
        for pattern in [
            "", "/", "/abs", "a//b", "./a", "a/../b", "a/[ab]", "{a,b}", "!a", "a**", "**b/c",
            "a\\b", "a\u{1b}b",
        ] {
            assert!(PathGlob::parse(pattern).is_err(), "{pattern:?}");
        }
    }
}

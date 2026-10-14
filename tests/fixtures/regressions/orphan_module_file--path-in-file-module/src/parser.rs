//! A file module, not a `mod.rs`: its plain declarations resolve under
//! `src/parser/`, while a `path` attribute resolves beside this file.

pub fn parse(text: &str) -> usize {
    text.len()
}

#[cfg(test)]
#[path = "parser_tests.rs"]
mod tests;

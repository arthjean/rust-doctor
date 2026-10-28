//! The non-ASCII table every text scanner is tested over.
//!
//! A scanner that adds 1 to the byte index of a character it found by a char
//! predicate slices inside the next multibyte character, and the
//! credential scanner did exactly that in front of `://`. The table puts each
//! of five awkward characters at every char boundary of one seed line per
//! shape the scanners look for, so an offset computed on bytes and used on a
//! `str` meets a multibyte character wherever it can land.
//!
//! Each owning module runs its scanners over [`lines`] in its own tests, since
//! most of them are private to it; the binary includes this file by path for
//! the terminal report's wrap. Every seed is assembled at run time, so no
//! source file of this repository carries a value the credential rule matches.

/// Two, three and four bytes wide, a combining sequence, and a double-width
/// character.
const INSERTED: [&str; 5] = ["é", "“", "𝄞", "e\u{301}", "日"];

/// One line per shape a scanner looks for: each credential pattern, a PEM
/// header, a fallback literal and a compiling workflow step.
fn seeds() -> Vec<String> {
    vec![
        format!("let key = \"AKIA{}\";", "Q3VJ7RNYH2KXW4PZ"),
        format!("token = ghp_{}", "a1".repeat(18)),
        format!("github_pat_{}", "B2".repeat(11)),
        format!("OPENAI=sk-{}", "c3".repeat(10)),
        format!("slack: xoxb-{}", "1234567890"),
        format!("url = \"postgres:/{}/app:Xk29fj3lQ@db.internal.example.net/prod\"", ""),
        format!("-----BEGIN {}PRIVATE KEY-----{}", "RSA ", "MIIE".repeat(16)),
        "let token = env::var(\"API_TOKEN\").unwrap_or(\"…\");".to_owned(),
        "run: cargo build # x".to_owned(),
    ]
}

/// Every seed with each inserted character at each of its char boundaries,
/// its end included.
pub(crate) fn lines() -> Vec<String> {
    let mut lines = Vec::new();
    for seed in seeds() {
        let boundaries = seed.char_indices().map(|(at, _)| at).chain([seed.len()]);
        for at in boundaries {
            for inserted in INSERTED {
                let mut line = seed.clone();
                line.insert_str(at, inserted);
                lines.push(line);
            }
        }
    }
    lines
}

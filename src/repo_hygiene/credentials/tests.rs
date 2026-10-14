// Every key-shaped value is assembled at runtime so that this source file
// never carries one the scan itself would report.

use super::*;

fn first(line: &str) -> Option<&'static str> {
    first_credential(line, &[]).map(|(name, _)| name)
}

/// An AWS access key id that is not the one AWS documents.
fn aws_key() -> String {
    format!("AKIA{}", "Q3VJ7RNYH2KXW4PZ")
}

#[test]
fn credential_matching_requires_the_boundary_and_the_minimum_tail() {
    let aws = aws_key();
    let (name, range) = first_credential(&format!("let key = \"{aws}\";"), &[]).unwrap();
    assert_eq!(name, "AWS access key id");
    assert_eq!(range, (11, 11 + aws.len()));

    assert_eq!(first(&format!("ghp_{}", "a".repeat(36))), Some("GitHub personal access token"));
    assert_eq!(first(&format!("github_pat_{}", "b".repeat(22))), Some("GitHub fine-grained token"));
    assert_eq!(first(&format!("sk-{}", "c".repeat(20))), Some("secret API key"));
    assert_eq!(first(&format!("xoxb-{}", "1".repeat(10))), Some("Slack bot token"));
    // Documentation spelling the pattern out, wildcard included, is prose:
    // a real PEM label carries only uppercase words.
    assert!(first(&format!("`-----BEGIN {} KEY-----`", "* PRIVATE")).is_none());

    // Boundary: the prefix inside a longer identifier is not a token.
    assert!(first(&format!("S{aws}")).is_none());
    assert!(first(&format!("risk-{}", "c".repeat(20))).is_none());
    // Minimum tail: a short suffix is prose, not a key.
    assert!(first("sk-learn").is_none());
    assert!(first("AKIA123").is_none());
    // Entropy alone never triggers.
    assert!(first("9f8e7d6c5b4a39281706f5e4d3c2b1a0").is_none());
}

#[test]
fn a_private_key_header_needs_a_key_body_under_it() {
    let header = format!("-----BEGIN {}-----", "PRIVATE KEY");
    let footer = format!("-----END {}-----", "PRIVATE KEY");
    let body = format!("MII{}", "Ev".repeat(30));

    // A parser keeping the header as a constant: nothing follows but its
    // footer and code.
    assert!(first(&header).is_none());
    assert!(first_credential(
        &format!("const HEADER: &str = \"{header}\";"),
        &[&format!("const FOOTER: &str = \"{footer}\";"), &body]
    )
    .is_none());
    assert!(first_credential(&header, &["fn parse(input: &str) -> Key {", "}"]).is_none());

    // A key: its body on the next non-empty lines, or on the header's own
    // line in a one-line literal.
    let rsa = format!("-----BEGIN {}-----", "RSA PRIVATE KEY");
    assert_eq!(
        first_credential(&rsa, &["", &body]).map(|(name, _)| name),
        Some("private key block")
    );
    assert_eq!(first(&format!("\"{header}\\n{body}\\n{footer}\"")), Some("private key block"));
    // A body past the lines a header is read with belongs to something else.
    assert!(first_credential(&header, &["a", "b", "c", &body]).is_none());
}

#[test]
fn placeholders_and_truncations_veto_a_match() {
    let documented = format!("AKIA{}", "IOSFODNN7EXAMPLE");
    assert!(first(&format!("let key = \"{documented}\";")).is_none());
    let aws = aws_key();
    assert!(first(&format!("// sample: {aws}")).is_none());
    assert!(first(&format!("let dummy_key = \"{aws}\";")).is_none());
    assert!(first(&format!("let key = \"{aws}...\";")).is_none());
    assert!(first(&format!("let key = \"{aws}\u{2026}\";")).is_none());
    // A word further than the window before the match is another sentence.
    assert_eq!(first(&format!("// example{}{aws}", " ".repeat(40))), Some("AWS access key id"));
    // A vetoed match gives way to a real one later on the line.
    let github = format!("ghp_{}", "a".repeat(36));
    let apart = " ".repeat(VETO_WINDOW);
    assert_eq!(first(&format!("\"{documented}\",{apart}\"{github}\"")), Some("GitHub personal access token"));
    // A placeholder header with a body under it.
    let header = format!("-----BEGIN {}-----", "PRIVATE KEY");
    let body = format!("MII{}", "Ev".repeat(30));
    assert!(first_credential(&format!("// example: {header}"), &[&body]).is_none());
}

#[test]
fn a_database_url_password_is_reported_only_where_it_reaches_a_deployment() {
    let secret = "Xk29fj3lQ";
    let line = format!("let url = \"postgres://app:{secret}@db.internal.example.net/prod\";");
    let (name, range) = first_credential(&line, &[]).unwrap();
    assert_eq!(name, "database URL password");
    assert_eq!(&line[range.0..range.1], secret);
    for scheme in ["postgresql", "mysql", "mongodb", "mongodb+srv", "redis", "POSTGRES"] {
        assert_eq!(
            first(&format!("{scheme}://app:{secret}@cache.prod.net:6379")),
            Some("database URL password"),
            "{scheme}"
        );
    }

    for password in ["pass", "changeme", "${DB_PASSWORD}", "$DB_PASSWORD", "<password>", "{{password}}", "{}", "{pass}"] {
        assert!(first(&format!("postgres://app:{password}@db.prod.net/app")).is_none(), "{password}");
    }
    for host in ["localhost", "LOCALHOST:5432", "db", "postgres:5432"] {
        assert!(first(&format!("postgres://app:{secret}@{host}/app")).is_none(), "{host}");
    }
    assert!(first(&format!("https://app:{secret}@api.service.net/")).is_none());
    assert!(first(&format!("my_postgres://app:{secret}@db.prod.net/")).is_none());
    assert!(first("postgres://app@db.prod.net/app").is_none());

    // Masking hides every password, a placeholder's included.
    assert_eq!(credential_ranges("postgres://user:pass@localhost/db"), vec![(16, 20)]);
}

const INSERTED: [&str; 5] = ["é", "“", "𝄞", "e\u{301}", "日"];

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

//! What the detectors read through a binding, and where they stop.
//!
//! Each case is one function the synthetic unit holds, scanned through the
//! registry the way a workspace unit is. A binding the detectors follow turns
//! the case positive; every form the resolver calls indeterminate leaves it
//! silent, which is the abstention the story asks of them.

use super::{REGISTERED, analyze};
use crate::policy::{SOURCE_DISABLED_TLS, SOURCE_DYNAMIC_SHELL};

fn shell(body: &str) -> Vec<&'static str> {
    analyze(&format!("fn run(user: &str, check: bool) {{\n{body}\n}}"), &REGISTERED).0
}

fn tls(source: &str) -> Vec<&'static str> {
    analyze(source, &REGISTERED).0
}

#[test]
fn a_let_bound_builder_is_followed_to_its_initializer() {
    assert_eq!(
        shell(
            "let mut cmd = std::process::Command::new(\"sh\");\n\
             cmd.arg(\"-c\").arg(format!(\"echo {user}\")).spawn();"
        ),
        [SOURCE_DYNAMIC_SHELL.id]
    );
    // From a nested block, which the `let` encloses.
    assert_eq!(
        shell(
            "let mut cmd = std::process::Command::new(\"sh\");\n\
             if check { cmd.arg(\"-c\").arg(format!(\"echo {user}\")); }"
        ),
        [SOURCE_DYNAMIC_SHELL.id]
    );
    // Through a chain of two bindings, and a rebinding that reads the one
    // before it.
    assert_eq!(
        shell(
            "let first = std::process::Command::new(\"sh\");\n\
             let mut cmd = first;\n\
             let payload = format!(\"echo {user}\");\n\
             cmd.arg(\"-c\").arg(payload);"
        ),
        [SOURCE_DYNAMIC_SHELL.id]
    );
}

#[test]
fn a_const_item_of_the_module_or_the_block_is_followed() {
    let module = "const INSECURE: bool = true;\n\
                  fn build() { let _ = http_client::Client::builder().danger_accept_invalid_certs(INSECURE); }";
    assert_eq!(tls(module), [SOURCE_DISABLED_TLS.id]);
    let block = "fn build() {\n\
                     const INSECURE: bool = true;\n\
                     let _ = http_client::Client::builder().danger_accept_invalid_certs(INSECURE);\n\
                 }";
    assert_eq!(tls(block), [SOURCE_DISABLED_TLS.id]);
    // A `const` whose value is `false` is read as `false`.
    let off = "const INSECURE: bool = false;\n\
               fn build() { let _ = http_client::Client::builder().danger_accept_invalid_certs(INSECURE); }";
    assert!(tls(off).is_empty());
    // The inline module is a scope of its own: the item outside it is not seen.
    let other_module = "const INSECURE: bool = true;\n\
                        mod inner { fn build() { let _ = http_client::Client::builder().danger_accept_invalid_certs(INSECURE); } }";
    assert!(tls(other_module).is_empty());
    // Two items of one name, which `cfg` makes legal, are indeterminate.
    let twice = "#[cfg(unix)] const INSECURE: bool = true;\n\
                 #[cfg(not(unix))] const INSECURE: bool = true;\n\
                 fn build() { let _ = http_client::Client::builder().danger_accept_invalid_certs(INSECURE); }";
    assert!(tls(twice).is_empty());
}

#[test]
fn every_binding_the_resolver_cannot_read_leaves_the_detectors_silent() {
    let silent = [
        // Reassigned after the `let`, plainly or compoundly.
        "let mut cmd = std::process::Command::new(\"sh\");\n\
         if check { cmd = std::process::Command::new(\"ls\"); }\n\
         cmd.arg(\"-c\").arg(format!(\"echo {user}\"));",
        // Bound by a pattern rather than by a plain name.
        "let (mut cmd, _) = (std::process::Command::new(\"sh\"), 0);\n\
         cmd.arg(\"-c\").arg(format!(\"echo {user}\"));",
        "let ref mut cmd = std::process::Command::new(\"sh\");\n\
         cmd.arg(\"-c\").arg(format!(\"echo {user}\"));",
        // Shadowed in a branch between the binding and the use.
        "let mut cmd = std::process::Command::new(\"sh\");\n\
         if check { let cmd = 0; let _ = cmd; }\n\
         cmd.arg(\"-c\").arg(format!(\"echo {user}\"));",
        // Shadowed by a closure parameter around the use.
        "let mut cmd = std::process::Command::new(\"sh\");\n\
         let run = |cmd: &mut std::process::Command| { cmd.arg(\"-c\").arg(format!(\"echo {user}\")); };\n\
         run(&mut cmd);",
        // Bound in a sibling block that does not enclose the use.
        "{ let mut cmd = std::process::Command::new(\"sh\"); }\n\
         cmd.arg(\"-c\").arg(format!(\"echo {user}\"));",
        // A `let ... else`, which binds through a refutable pattern.
        "let Some(mut cmd) = Some(std::process::Command::new(\"sh\")) else { return };\n\
         cmd.arg(\"-c\").arg(format!(\"echo {user}\"));",
    ];
    for body in silent {
        assert!(shell(body).is_empty(), "a detector read through:\n{body}");
    }

    // Taken from a parameter.
    assert!(
        tls("fn build(insecure: bool) { let _ = http_client::Client::builder().danger_accept_invalid_certs(insecure); }")
            .is_empty()
    );
    assert!(
        analyze(
            "fn run(cmd: &mut std::process::Command, user: &str) { cmd.arg(\"-c\").arg(format!(\"echo {user}\")); }",
            &REGISTERED
        )
        .0
        .is_empty()
    );
    // A compound assignment leaves a flag nobody can read off its `let`.
    assert!(
        tls("fn build(check: bool) { let mut insecure = true; insecure &= check; \
             let _ = http_client::Client::builder().danger_accept_invalid_certs(insecure); }")
            .is_empty()
    );
}

#[test]
fn a_chain_of_bindings_past_the_depth_bound_is_abstained_on() {
    let mut body = String::from("let b0 = std::process::Command::new(\"sh\");\n");
    for index in 1..=8 {
        body.push_str(&format!("let b{index} = b{};\n", index - 1));
    }
    body.push_str("let mut cmd = b8;\ncmd.arg(\"-c\").arg(format!(\"echo {user}\"));");
    assert!(shell(&body).is_empty());

    let mut shallow = String::from("let b0 = std::process::Command::new(\"sh\");\n");
    shallow.push_str("let mut cmd = b0;\ncmd.arg(\"-c\").arg(format!(\"echo {user}\"));");
    assert_eq!(shell(&shallow), [SOURCE_DYNAMIC_SHELL.id]);
}

#[test]
fn constant_arguments_keep_a_format_string_constant() {
    // A captured `const`, a positional one, and one from another module
    // spelled the way a constant is: none of them is a runtime value.
    for body in [
        "const GREETING: &str = \"hi\";\n\
         let _ = std::process::Command::new(\"sh\").arg(\"-c\").arg(format!(\"echo {GREETING}\"));",
        "const GREETING: &str = \"hi\";\n\
         let _ = std::process::Command::new(\"sh\").arg(\"-c\").arg(format!(\"echo {}\", GREETING));",
        "let _ = std::process::Command::new(\"sh\").arg(\"-c\").arg(format!(\"echo {}\", texts::GREETING));",
        "let greeting = \"hi\";\n\
         let _ = std::process::Command::new(\"sh\").arg(\"-c\").arg(format!(\"echo {greeting}\"));",
        "let _ = std::process::Command::new(\"sh\").arg(\"-c\").arg(\"echo \".to_owned() + \"hi\");",
    ] {
        assert!(shell(body).is_empty(), "a constant read as dynamic:\n{body}");
    }
}

//! A shell command string assembled from a runtime value.
//!
//! `Command::new("sh").arg("-c").arg(script)` hands `script` to a shell, which
//! parses it: a value interpolated into it can end the command and start
//! another. The same holds for `cmd /C`, `powershell -Command` and the shell
//! named by its path, and for the argv written as one `.args([..])` array. A
//! program name assembled from a runtime value is the other way in, since it
//! chooses what runs. A dynamic argument of a fixed program that is not a shell
//! is one argv element, which nothing parses as a command, and stays silent.
//!
//! The rule carries no test-code exemption on purpose, because an interpolated
//! shell command is a finding wherever it is written, and a test harness is a
//! place it runs.

use ra_ap_syntax::ast::{self, HasArgList};
use ra_ap_syntax::{AstNode, SyntaxKind, SyntaxNode};

use super::{CHAIN_DEPTH, Context, Detection, Detector, Item, dynamic_text, only_argument, strip_wrappers};
use crate::policy::SOURCE_DYNAMIC_SHELL;
use crate::source_kernel::literal_string;
use crate::source_text::intersects_errors;

pub(in crate::source_kernel) static DYNAMIC_SHELL: Detector = Detector {
    definition: &SOURCE_DYNAMIC_SHELL,
    node: SyntaxKind::METHOD_CALL_EXPR,
    inspect: dynamic_shell,
};

pub(in crate::source_kernel) static TAINTED_PROGRAM: Detector = Detector {
    definition: &SOURCE_DYNAMIC_SHELL,
    node: SyntaxKind::CALL_EXPR,
    inspect: tainted_program,
};

static COMMAND: Item = Item {
    krate: "std",
    segments: &["process", "Command"],
};

/// Programs that parse their command argument as a script, by lowercase file
/// name, so `/bin/sh` and `C:\Windows\System32\CMD.EXE` name one, each with
/// the flags after which it reads the next argument as that script. A flag
/// another shell family uses is not one: `cmd` takes `/C`, never `-c`.
const SHELLS: [(&str, &[&str]); 10] = [
    ("bash", POSIX_FLAGS),
    ("cmd", CMD_FLAGS),
    ("cmd.exe", CMD_FLAGS),
    ("dash", POSIX_FLAGS),
    ("ksh", POSIX_FLAGS),
    ("powershell", POWERSHELL_FLAGS),
    ("powershell.exe", POWERSHELL_FLAGS),
    ("pwsh", POWERSHELL_FLAGS),
    ("sh", POSIX_FLAGS),
    ("zsh", POSIX_FLAGS),
];

const POSIX_FLAGS: &[&str] = &["-c"];
const CMD_FLAGS: &[&str] = &["/C", "/c"];
/// PowerShell accepts any unambiguous prefix of a parameter name, and `-c` is
/// the one its own help documents for `-Command`.
const POWERSHELL_FLAGS: &[&str] = &["-Command", "-c"];

#[cfg(test)]
pub(in crate::source_kernel) static VARIANTS: super::Variants = super::Variants {
    rule: SOURCE_DYNAMIC_SHELL.id,
    positive: super::Case {
        asynchronous: false,
        import: "std::process::Command",
        parameters: "$user: &str",
        setup: "",
        receiver: "{Item}::new(\"sh\")",
        chain: ".arg(\"-c\").arg(format!(\"echo {$user}\"))",
    },
    negative: super::Case {
        asynchronous: false,
        import: "std::process::Command",
        parameters: "$user: &str",
        setup: "",
        receiver: "{Item}::new(\"sh\")",
        chain: ".arg(\"-c\").arg(\"echo literal\").arg($user)",
    },
    interpolates_parameters: true,
    known_misses: &[],
    semantic: &[
        super::Semantic {
            description: "the flag and the script passed together through args",
            source: "use std::process::Command;\nfn run(user: &str) { let _ = Command::new(\"sh\").args([\"-c\", &format!(\"echo {user}\")]).status(); }",
            fires: true,
            carve_out: "",
        },
        super::Semantic {
            description: "the flag passed through args and the script through arg",
            source: "use std::process::Command;\nfn run(user: &str) { let _ = Command::new(\"bash\").args([\"-c\"]).env(\"LANG\", \"C\").arg(format!(\"echo {user}\")).status(); }",
            fires: true,
            carve_out: "",
        },
        super::Semantic {
            description: "the shell named by its absolute path",
            source: "use std::process::Command;\nfn run(user: &str) { let _ = Command::new(\"/bin/sh\").arg(\"-c\").arg(format!(\"echo {user}\")).status(); }",
            fires: true,
            carve_out: "",
        },
        super::Semantic {
            description: "cmd /C on Windows",
            source: "use std::process::Command;\nfn run(user: &str) { let _ = Command::new(\"cmd\").arg(\"/C\").arg(format!(\"echo {user}\")).status(); }",
            fires: true,
            carve_out: "",
        },
        super::Semantic {
            description: "powershell -Command",
            source: "use std::process::Command;\nfn run(user: &str) { let _ = Command::new(\"powershell.exe\").arg(\"-Command\").arg(format!(\"Write-Output {user}\")).status(); }",
            fires: true,
            carve_out: "",
        },
        super::Semantic {
            description: "a program name assembled from a runtime value",
            source: "use std::process::Command;\nfn run(tool: &str) { let _ = Command::new(format!(\"/opt/{tool}/bin/run\")).status(); }",
            fires: true,
            carve_out: "",
        },
        super::Semantic {
            description: "a script path handed to a shell without a script flag",
            source: "use std::process::Command;\nfn run(script: &str) { let _ = Command::new(\"sh\").arg(format!(\"{script}.sh\")).status(); }",
            fires: false,
            carve_out: "without -c the shell reads the argument as a file to run, not as a command line",
        },
        super::Semantic {
            description: "a dynamic argument of a fixed program that is not a shell",
            source: "use std::process::Command;\nfn run(user: &str) { let _ = Command::new(\"git\").arg(\"log\").arg(format!(\"--author={user}\")).status(); }",
            fires: false,
            carve_out: "a discrete argv element of a program that is not a shell is never parsed as a command",
        },
        super::Semantic {
            description: "a number interpolated into the script",
            source: "use std::process::Command;\nfn pause(secs: u32) { let _ = Command::new(\"sh\").arg(\"-c\").arg(format!(\"sleep {secs}\")).status(); }",
            fires: false,
            carve_out: "a parameter typed as a numeric primitive cannot spell a shell command",
        },
        super::Semantic {
            description: "a dynamic argument of a fixed program, passed through args",
            source: "use std::process::Command;\nfn run(user: &str) { let _ = Command::new(\"git\").args([\"-c\", &format!(\"user.name={user}\")]).status(); }",
            fires: false,
            carve_out: "git's -c sets a configuration value; only a shell reads the next argument as a script",
        },
    ],
};

/// `.arg(x)` or `.args([.., x, ..])` where `x` is dynamic text that follows a
/// script flag on a command whose program is a shell.
fn dynamic_shell(context: &Context<'_>, node: &SyntaxNode) -> Option<Detection> {
    let call = ast::MethodCallExpr::cast(node.clone())?;
    let own = arguments(&call)?;
    // The arguments first: a call adding no dynamic text never pays for
    // reading the chain behind it.
    if !own.iter().any(|element| dynamic_text(context, element)) {
        return None;
    }
    let (flags, before) = argv(context, call.receiver()?)?;
    let payload = own.iter().enumerate().find_map(|(index, element)| {
        let previous = match index.checked_sub(1) {
            Some(previous) => own.get(previous),
            None => before.last(),
        }?;
        let flag = literal_string(strip_wrappers(previous.clone()))?;
        (flags.contains(&flag.as_str()) && dynamic_text(context, element)).then_some(element)
    })?;
    if intersects_errors(call.syntax().text_range(), context.error_ranges) {
        return None;
    }
    Some(Detection {
        message: "A dynamic value is interpolated into a shell command string.".into(),
        range: payload.syntax().text_range(),
    })
}

/// `Command::new(x)` where `x` is dynamic text: the program itself.
fn tainted_program(context: &Context<'_>, node: &SyntaxNode) -> Option<Detection> {
    let call = ast::CallExpr::cast(node.clone())?;
    // The name, then the argument: most calls are not a constructor taking
    // one assembled string, and those never pay for resolving their path.
    let ast::Expr::PathExpr(callee) = call.expr()? else {
        return None;
    };
    if callee.path()?.segment()?.name_ref()?.text() != "new" {
        return None;
    }
    let program = only_argument(call.arg_list()?)?;
    if !dynamic_text(context, &program)
        || !context.calls(&call.expr()?, &COMMAND, "new")
        || intersects_errors(call.syntax().text_range(), context.error_ranges)
    {
        return None;
    }
    Some(Detection {
        message: "The program a command runs is assembled from a runtime value.".into(),
        range: program.syntax().text_range(),
    })
}

/// The argv elements one `.arg(..)` or `.args([..])` call adds, in order.
/// An `args` of anything but an array literal cannot be read.
fn arguments(call: &ast::MethodCallExpr) -> Option<Vec<ast::Expr>> {
    let list = call.arg_list()?;
    match call.name_ref()?.text().as_str() {
        "arg" => only_argument(list).map(|argument| vec![argument]),
        "args" => match strip_wrappers(only_argument(list)?) {
            ast::Expr::ArrayExpr(array) => Some(array.exprs().collect()),
            _ => None,
        },
        _ => None,
    }
}

/// The script flags of the shell a command chain starts from,
/// `Command::new(<shell>)`, and the argv the chain holds up to `receiver`,
/// read through the bindings it was stored in. Methods that add no argument,
/// `env` or `current_dir`, are passed over; an `args` that cannot be read, or a
/// program that is not a shell, is `None`.
fn argv(context: &Context<'_>, receiver: ast::Expr) -> Option<(&'static [&'static str], Vec<ast::Expr>)> {
    let mut calls = Vec::new();
    let mut expression = receiver;
    for _ in 0..CHAIN_DEPTH {
        match context.value(strip_wrappers(expression))? {
            ast::Expr::MethodCallExpr(call) => {
                expression = call.receiver()?;
                calls.push(call);
            }
            ast::Expr::CallExpr(command) => {
                if !context.calls(&command.expr()?, &COMMAND, "new") {
                    return None;
                }
                let program = only_argument(command.arg_list()?).and_then(literal_string)?;
                let name = program
                    .rsplit(['/', '\\'])
                    .next()
                    .unwrap_or(&program)
                    .to_ascii_lowercase();
                let (_, flags) = SHELLS.iter().find(|(shell, _)| *shell == name)?;
                let mut elements = Vec::new();
                for call in calls.iter().rev() {
                    if matches!(call.name_ref()?.text().as_str(), "arg" | "args") {
                        elements.extend(arguments(call)?);
                    }
                }
                return Some((flags, elements));
            }
            _ => return None,
        }
    }
    None
}

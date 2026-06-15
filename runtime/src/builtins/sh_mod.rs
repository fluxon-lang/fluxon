// ---------------- sh (external shell commands) ----------------
//
// sh.run cmd -> {stdout: str  stderr: str  code: int}.
// The command is run through the SHELL (Unix: `sh -c`, Windows: `cmd /C`) — so
// shell features like `cd x && cargo build`, pipes (`|`), `&&`, glob work (in
// issue #26 Sonnet guessed exactly this pattern). Needed for a coding agent,
// CI scripts, build automation.
//
// `code == 0` is success (the Unix convention). If the process dies from a signal
// (no exit code on Unix) code = -1. The command ITSELF failing (a non-zero code)
// is NOT a Flow::err — that is an expected result, the caller checks via `code`.
// Only when the process cannot be started at all (e.g. the shell is not found) Flow::err.
//
// Blocking dangerous commands is deliberately ABSENT — that is the user's responsibility (issue #26).
use std::collections::BTreeMap;

use crate::builtins::R;
use crate::builtins::args::*;
use crate::interp::Flow;
use crate::value::Value;

pub(crate) fn sh_module(func: &str, args: Vec<Value>) -> R {
    match func {
        "run" => {
            let cmd = arg_str(&args, 0, "sh.run")?;
            let mut command;
            #[cfg(windows)]
            {
                command = std::process::Command::new("cmd");
                command.arg("/C").arg(&cmd);
            }
            #[cfg(not(windows))]
            {
                command = std::process::Command::new("sh");
                command.arg("-c").arg(&cmd);
            }
            let output = command
                .output()
                .map_err(|e| Flow::err(format!("sh.run: could not start command: {}", e)))?;
            // Read stdout/stderr as lossy UTF-8 — no panic even on binary output
            // (unlike the json decoder, there is no text guarantee here).
            let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
            let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
            // a process ended by a signal has code None -> -1.
            let code = output.status.code().unwrap_or(-1) as i64;
            let mut m = BTreeMap::new();
            m.insert("stdout".to_string(), Value::Str(stdout));
            m.insert("stderr".to_string(), Value::Str(stderr));
            m.insert("code".to_string(), Value::Int(code));
            Ok(Value::Map(m))
        }
        _ => Err(Flow::err(format!("sh module has no function '{}'", func))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::builtins::is_module;

    // Get the command fields as text (to simplify the error messages).
    fn run(cmd: &str) -> BTreeMap<String, Value> {
        match sh_module("run", vec![Value::Str(cmd.into())]) {
            Ok(Value::Map(m)) => m,
            other => panic!("sh.run must return a map, got {:?}", other.is_ok()),
        }
    }

    // Simple echo: stdout correct, code 0, stderr empty.
    #[test]
    fn echo_stdout_and_code() {
        let m = run("echo hello");
        match m.get("stdout") {
            Some(Value::Str(s)) => assert_eq!(s.trim_end(), "hello"),
            _ => panic!("stdout must be str"),
        }
        assert!(matches!(m.get("code"), Some(Value::Int(0))));
        match m.get("stderr") {
            Some(Value::Str(s)) => assert!(s.is_empty()),
            _ => panic!("stderr must be str"),
        }
    }

    // Non-zero exit: the command failed -> NOT Flow::err, code != 0.
    #[test]
    fn nonzero_exit_is_not_error() {
        let m = run("exit 3");
        assert!(matches!(m.get("code"), Some(Value::Int(3))));
    }

    // stderr is captured separately (does not mix with stdout).
    #[test]
    fn stderr_captured_separately() {
        let m = run("echo error 1>&2");
        match m.get("stderr") {
            Some(Value::Str(s)) => assert_eq!(s.trim_end(), "error"),
            _ => panic!("stderr must be str"),
        }
        match m.get("stdout") {
            Some(Value::Str(s)) => assert!(s.is_empty()),
            _ => panic!("stdout must be str"),
        }
    }

    // Shell features (`&&`, pipe) work — the command goes through the shell.
    #[test]
    fn shell_features_work() {
        let m = run("echo one && echo two");
        match m.get("stdout") {
            Some(Value::Str(s)) => {
                assert!(s.contains("one") && s.contains("two"));
            }
            _ => panic!("stdout must be str"),
        }
        assert!(matches!(m.get("code"), Some(Value::Int(0))));
    }

    // An unknown sh function returns an explicit error.
    #[test]
    fn unknown_func_errors() {
        match sh_module("nope", vec![]) {
            Err(Flow::Error(msg)) => assert!(msg.contains("sh module")),
            _ => panic!("expected Flow::Error"),
        }
    }

    // sh must be recognized as a module.
    #[test]
    fn sh_is_module() {
        assert!(is_module("sh"));
    }
}

// ---------------- io ----------------
// Terminal input/output. `log` always appends `\n` to stderr; an interactive CLI
// (REPL, agent, wizard) needs to read from stdin and a prompt without `\n`. The
// prompt and input go through stdout/stdin (log is stderr — they must not mix).
use crate::builtins::R;
use crate::builtins::args::*;
use crate::interp::Flow;
use crate::value::Value;

pub(crate) fn io_module(func: &str, args: Vec<Value>) -> R {
    use std::io::Write;
    match func {
        // io.read_line -> a single line from stdin (the trailing \n is removed).
        // EOF (Ctrl-D, end of a pipe) -> nil, so the caller stops the loop.
        "read_line" => {
            let mut line = String::new();
            match std::io::stdin().read_line(&mut line) {
                Ok(0) => Ok(Value::Nil),
                Ok(_) => {
                    // strip the trailing \n (and Windows \r)
                    let trimmed = line.trim_end_matches(['\n', '\r']);
                    Ok(Value::Str(trimmed.to_string()))
                }
                Err(e) => Err(Flow::err(format!("io.read_line: {}", e))),
            }
        }
        // io.print s -> write to stdout WITHOUT a \n (to show a prompt).
        // Flush immediately — otherwise the prompt stays in the buffer and the user
        // does not see it before typing input.
        "print" => {
            let s = arg_str(&args, 0, "io.print")?;
            let mut out = std::io::stdout();
            out.write_all(s.as_bytes())
                .and_then(|_| out.flush())
                .map_err(|e| Flow::err(format!("io.print: {}", e)))?;
            Ok(Value::Nil)
        }
        // io.prompt msg -> prints msg without a \n, then reads a single line.
        // A convenient shorthand for io.print + io.read_line.
        "prompt" => {
            let s = arg_str(&args, 0, "io.prompt")?;
            let mut out = std::io::stdout();
            out.write_all(s.as_bytes())
                .and_then(|_| out.flush())
                .map_err(|e| Flow::err(format!("io.prompt: {}", e)))?;
            io_module("read_line", vec![])
        }
        _ => Err(Flow::err(format!("io module has no function '{}'", func))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::builtins::is_module;

    // io.print returns nil as a value (writing to stdout is a side effect).
    // The test writes "" (empty) to stdout — does not pollute the observed output.
    // (Value/Flow do not derive Debug — match instead of unwrap.)
    #[test]
    fn print_returns_nil() {
        match io_module("print", vec![Value::Str(String::new())]) {
            Ok(Value::Nil) => {}
            _ => panic!("io.print must return nil"),
        }
    }

    // io.print requires the argument to be str.
    #[test]
    fn print_requires_str_arg() {
        assert!(io_module("print", vec![Value::Int(5)]).is_err());
        assert!(io_module("print", vec![]).is_err());
    }

    // An unknown io function returns an explicit error. (Flow does not derive Debug
    // — we reach the error text via the Flow::Error inside.)
    #[test]
    fn unknown_func_errors() {
        match io_module("nope", vec![]) {
            Err(Flow::Error(msg)) => assert!(msg.contains("io module")),
            _ => panic!("expected Flow::Error"),
        }
    }

    // io must be recognized as a module (the argument-less Field dispatch relies on this).
    #[test]
    fn io_is_module() {
        assert!(is_module("io"));
    }
}

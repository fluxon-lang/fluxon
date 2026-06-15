// ---------------- fs (local file system) ----------------
//
// Convention: on success a useful value (text/bool/list) or the :ok sym; on a
// real IO error a Flow::err — so the cause is not lost (the io battery is like this).
// The only exception: fs.read returns nil when the file is missing (an expected
// case, not an error — handy for folding the "does the file exist?" check into read).
use std::sync::Arc;

use crate::builtins::R;
use crate::builtins::args::*;
use crate::interp::Flow;
use crate::value::Value;

pub(crate) fn fs_module(func: &str, args: Vec<Value>) -> R {
    match func {
        // fs.read path -> the file text (str), or nil if the file is missing.
        // Flow::err on a non-UTF-8 file or a permission error.
        "read" => {
            let path = arg_str(&args, 0, "fs.read")?;
            match std::fs::read_to_string(&path) {
                Ok(s) => Ok(Value::Str(s)),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Value::Nil),
                Err(e) => Err(Flow::err(format!("fs.read {}: {}", path, e))),
            }
        }
        // fs.readb path -> the file bytes (bytes), or nil if missing. The binary
        // counterpart of fs.read (issue #132) — non-UTF-8 files like images/PDFs
        // error in fs.read, and are read through this instead.
        "readb" => {
            let path = arg_str(&args, 0, "fs.readb")?;
            match std::fs::read(&path) {
                Ok(b) => Ok(Value::Bytes(Arc::new(b))),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Value::Nil),
                Err(e) => Err(Flow::err(format!("fs.readb {}: {}", path, e))),
            }
        }
        // fs.write path content -> overwrites the file (previous content is lost).
        // Intermediate directories must exist (use fs.mkdirp if needed).
        // content is str OR bytes — no separate "writeb" is needed for writing,
        // because the source type does not change the path (unlike reading).
        "write" => {
            let path = arg_str(&args, 0, "fs.write")?;
            let content = arg_bytes(&args, 1, "fs.write")?;
            std::fs::write(&path, content.as_slice())
                .map_err(|e| Flow::err(format!("fs.write {}: {}", path, e)))?;
            Ok(Value::Sym("ok".into()))
        }
        // fs.append path content -> appends to the end of an existing file (creates
        // it if missing). content is str or bytes (same as fs.write).
        "append" => {
            use std::io::Write;
            let path = arg_str(&args, 0, "fs.append")?;
            let content = arg_bytes(&args, 1, "fs.append")?;
            let mut f = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&path)
                .map_err(|e| Flow::err(format!("fs.append {}: {}", path, e)))?;
            f.write_all(content.as_slice())
                .map_err(|e| Flow::err(format!("fs.append {}: {}", path, e)))?;
            Ok(Value::Sym("ok".into()))
        }
        // fs.exists path -> bool (whether a file OR directory exists).
        "exists" => {
            let path = arg_str(&args, 0, "fs.exists")?;
            Ok(Value::Bool(std::path::Path::new(&path).exists()))
        }
        // fs.ls path -> a list of names inside the directory [str] (just the name,
        // not the full path). Sorted so the order is deterministic.
        "ls" => {
            let path = arg_str(&args, 0, "fs.ls")?;
            let entries = std::fs::read_dir(&path)
                .map_err(|e| Flow::err(format!("fs.ls {}: {}", path, e)))?;
            let mut names = Vec::new();
            for entry in entries {
                let entry = entry.map_err(|e| Flow::err(format!("fs.ls {}: {}", path, e)))?;
                names.push(entry.file_name().to_string_lossy().into_owned());
            }
            names.sort();
            Ok(Value::List(names.into_iter().map(Value::Str).collect()))
        }
        // fs.del path -> deletes a file or an empty directory -> :ok.
        // If the directory is not empty, Flow::err (recursive delete is deliberately
        // absent — safer, so a whole tree is not accidentally removed).
        "del" => {
            let path = arg_str(&args, 0, "fs.del")?;
            let p = std::path::Path::new(&path);
            let res = if p.is_dir() {
                std::fs::remove_dir(p)
            } else {
                std::fs::remove_file(p)
            };
            res.map_err(|e| Flow::err(format!("fs.del {}: {}", path, e)))?;
            Ok(Value::Sym("ok".into()))
        }
        // fs.mkdirp path -> creates the directory (with the needed intermediate dirs) -> :ok.
        // Not an error if the directory already exists (idempotent).
        "mkdirp" => {
            let path = arg_str(&args, 0, "fs.mkdirp")?;
            std::fs::create_dir_all(&path)
                .map_err(|e| Flow::err(format!("fs.mkdirp {}: {}", path, e)))?;
            Ok(Value::Sym("ok".into()))
        }
        _ => Err(Flow::err(format!("fs module has no function '{}'", func))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::builtins::is_module;

    // A unique temporary directory per test (so they do not collide with other tests).
    // Process pid + test name is unique enough — even if tests run in parallel.
    fn tmp_dir(tag: &str) -> std::path::PathBuf {
        let mut p = std::env::temp_dir();
        p.push(format!("fluxon_fs_test_{}_{}", std::process::id(), tag));
        let _ = std::fs::remove_dir_all(&p); // clean up previous leftover
        std::fs::create_dir_all(&p).expect("tmp dir not created");
        p
    }

    fn path_str(dir: &std::path::Path, name: &str) -> String {
        dir.join(name).to_string_lossy().into_owned()
    }

    // write + read round-trip: the written text is read back exactly.
    #[test]
    fn write_then_read() {
        let dir = tmp_dir("write_read");
        let f = path_str(&dir, "a.txt");
        match fs_module(
            "write",
            vec![Value::Str(f.clone()), Value::Str("hello".into())],
        ) {
            Ok(Value::Sym(s)) if s == "ok" => {}
            _ => panic!("fs.write must return :ok"),
        }
        match fs_module("read", vec![Value::Str(f)]) {
            Ok(Value::Str(s)) => assert_eq!(s, "hello"),
            _ => panic!("fs.read must return the written text"),
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    // Reading a missing file returns nil (not an error) — an issue requirement.
    #[test]
    fn read_missing_is_nil() {
        let dir = tmp_dir("read_missing");
        let f = path_str(&dir, "missing.txt");
        match fs_module("read", vec![Value::Str(f)]) {
            Ok(Value::Nil) => {}
            _ => panic!("missing file must return nil"),
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    // append creates a missing file and appends successively.
    #[test]
    fn append_accumulates() {
        let dir = tmp_dir("append");
        let f = path_str(&dir, "log.txt");
        let _ = fs_module(
            "append",
            vec![Value::Str(f.clone()), Value::Str("a".into())],
        );
        let _ = fs_module(
            "append",
            vec![Value::Str(f.clone()), Value::Str("b".into())],
        );
        match fs_module("read", vec![Value::Str(f)]) {
            Ok(Value::Str(s)) => assert_eq!(s, "ab"),
            _ => panic!("append must accumulate text"),
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    // exists: an existing file is true, a missing file is false.
    #[test]
    fn exists_reflects_reality() {
        let dir = tmp_dir("exists");
        let f = path_str(&dir, "present.txt");
        let _ = fs_module("write", vec![Value::Str(f.clone()), Value::Str("x".into())]);
        match fs_module("exists", vec![Value::Str(f)]) {
            Ok(Value::Bool(true)) => {}
            _ => panic!("existing file must be true"),
        }
        match fs_module("exists", vec![Value::Str(path_str(&dir, "missing.txt"))]) {
            Ok(Value::Bool(false)) => {}
            _ => panic!("missing file must be false"),
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ls: returns the names inside the directory in sorted order.
    #[test]
    fn ls_lists_sorted_names() {
        let dir = tmp_dir("ls");
        let _ = fs_module(
            "write",
            vec![Value::Str(path_str(&dir, "b.txt")), Value::Str("".into())],
        );
        let _ = fs_module(
            "write",
            vec![Value::Str(path_str(&dir, "a.txt")), Value::Str("".into())],
        );
        match fs_module("ls", vec![Value::Str(dir.to_string_lossy().into_owned())]) {
            Ok(Value::List(items)) => {
                let names: Vec<String> = items
                    .iter()
                    .map(|v| match v {
                        Value::Str(s) => s.clone(),
                        _ => panic!("ls must return a list of str"),
                    })
                    .collect();
                assert_eq!(names, vec!["a.txt".to_string(), "b.txt".to_string()]);
            }
            _ => panic!("ls must return a list"),
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    // del: deletes the file, then exists becomes false.
    #[test]
    fn del_removes_file() {
        let dir = tmp_dir("del");
        let f = path_str(&dir, "o.txt");
        let _ = fs_module("write", vec![Value::Str(f.clone()), Value::Str("x".into())]);
        match fs_module("del", vec![Value::Str(f.clone())]) {
            Ok(Value::Sym(s)) if s == "ok" => {}
            _ => panic!("fs.del must return :ok"),
        }
        match fs_module("exists", vec![Value::Str(f)]) {
            Ok(Value::Bool(false)) => {}
            _ => panic!("deleted file must not exist"),
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    // mkdirp: creates the directory recursively and is idempotent (:ok the second time too).
    #[test]
    fn mkdirp_recursive_and_idempotent() {
        let dir = tmp_dir("mkdirp");
        let nested = path_str(&dir, "a/b/c");
        match fs_module("mkdirp", vec![Value::Str(nested.clone())]) {
            Ok(Value::Sym(s)) if s == "ok" => {}
            _ => panic!("fs.mkdirp must return :ok"),
        }
        assert!(std::path::Path::new(&nested).is_dir());
        // the second call must not error (idempotent)
        match fs_module("mkdirp", vec![Value::Str(nested)]) {
            Ok(Value::Sym(s)) if s == "ok" => {}
            _ => panic!("mkdirp must be idempotent"),
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    // An unknown fs function returns an explicit error.
    #[test]
    fn unknown_func_errors() {
        match fs_module("nope", vec![]) {
            Err(Flow::Error(msg)) => assert!(msg.contains("fs module")),
            _ => panic!("expected Flow::Error"),
        }
    }

    // fs must be recognized as a module.
    #[test]
    fn fs_is_module() {
        assert!(is_module("fs"));
    }

    // Binary round-trip (issue #132): bytes are written, fs.readb returns exactly
    // those bytes — non-UTF-8 content is not corrupted either.
    #[test]
    fn write_bytes_then_readb() {
        let dir = tmp_dir("write_readb");
        let f = path_str(&dir, "bin.dat");
        let data = vec![0xff, 0x00, 0xfe, 0x88, 0x01];
        match fs_module(
            "write",
            vec![Value::Str(f.clone()), Value::Bytes(Arc::new(data.clone()))],
        ) {
            Ok(Value::Sym(s)) if s == "ok" => {}
            _ => panic!("fs.write with bytes must return :ok"),
        }
        match fs_module("readb", vec![Value::Str(f.clone())]) {
            Ok(Value::Bytes(b)) => assert_eq!(*b, data),
            _ => panic!("fs.readb must return bytes"),
        }
        // A text file is read with readb too (its bytes).
        match fs_module("read", vec![Value::Str(f)]) {
            Err(Flow::Error(_)) => {} // not UTF-8 — fs.read returns an explicit error
            _ => panic!("fs.read must error on a non-UTF-8 file"),
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    // fs.readb returns nil for a missing file (symmetric with fs.read).
    #[test]
    fn readb_missing_is_nil() {
        let dir = tmp_dir("readb_missing");
        match fs_module("readb", vec![Value::Str(path_str(&dir, "missing.bin"))]) {
            Ok(Value::Nil) => {}
            _ => panic!("missing file must return nil"),
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    // fs.append works with bytes too (mixed str + bytes writing).
    #[test]
    fn append_bytes() {
        let dir = tmp_dir("append_bytes");
        let f = path_str(&dir, "mix.dat");
        let _ = fs_module(
            "write",
            vec![Value::Str(f.clone()), Value::Str("ab".into())],
        );
        let _ = fs_module(
            "append",
            vec![Value::Str(f.clone()), Value::Bytes(Arc::new(vec![0xff]))],
        );
        match fs_module("readb", vec![Value::Str(f)]) {
            Ok(Value::Bytes(b)) => assert_eq!(*b, vec![b'a', b'b', 0xff]),
            _ => panic!("fs.readb must return bytes"),
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}

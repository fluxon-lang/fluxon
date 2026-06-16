// ---------------- tui ----------------
// Terminal UI for CLI tools and agents. Two layers, all under one namespace so the
// AI learns one battery:
//
//   * styling (str -> str): `tui.green s`, `tui.bold s`, `tui.dim s`, ... — wraps
//     text in ANSI escapes. Pure, side-effect-free; returns a string you can `log`,
//     interpolate or concatenate. When stdout is NOT a tty (piped, redirected to a
//     file) the codes are dropped — output stays clean for grep/files.
//
//   * text widgets (-> str): `tui.rule`, `tui.box`, `tui.badge`, `tui.table` —
//     render a string. Still pure (no I/O), the caller decides where it goes.
//
//   * interactive widgets (I/O): `tui.input`, `tui.password`, `tui.confirm`,
//     `tui.select`, `tui.checkbox` — read keys and draw to the terminal. The
//     arrow-key ones (select/checkbox) enter raw mode via crossterm and ALWAYS
//     restore it (even on error) so they never wedge the terminal.
//
// A spinner (animate while a lambda runs) is deliberately left out here: running a
// Fluxon lambda needs the interpreter, which `call_module` does not get — it would
// have to be wired at the interp level like `http.serve`. Tracked as a follow-up.
//
// This grew out of issue #28 (`ansi.<color>`): the experiment showed an agent wants
// terminal UX as part of the task, not just colors — so colors live here next to the
// widgets a CLI agent actually composes (prompt -> select -> spinner -> table).
use std::io::{IsTerminal, Write};

use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use crossterm::terminal;

use crate::builtins::R;
use crate::builtins::args::*;
use crate::interp::Flow;
use crate::value::Value;

// ANSI SGR codes. `\x1b[<code>m ... \x1b[0m`. Kept tiny on purpose.
const RESET: &str = "\x1b[0m";

// Should we emit color? Default: only on a real tty (a pipe/file gets clean text so
// logs and `| grep` stay intact). Two standard overrides, checked first:
//   NO_COLOR (any value)      -> never color (the cross-tool convention, no-color.org)
//   CLICOLOR_FORCE=1 / FORCE_COLOR -> always color, even through a pipe (for demos,
//                                     recording asciinema, `… | less -R`).
fn color_enabled() -> bool {
    if std::env::var_os("NO_COLOR").is_some() {
        return false;
    }
    if matches!(std::env::var("CLICOLOR_FORCE").as_deref(), Ok("1"))
        || std::env::var_os("FORCE_COLOR").is_some()
    {
        return true;
    }
    std::io::stdout().is_terminal()
}

// Wrap `s` with the SGR code, but only when a tty is attached. `code` is the SGR
// number (e.g. "32" green, "1" bold). The reset closes ALL attributes — fine here
// because we never nest our own codes (the AI composes by concatenation, not nesting).
fn style(code: &str, s: &str) -> Value {
    if color_enabled() {
        Value::Str(format!("\x1b[{}m{}{}", code, s, RESET))
    } else {
        Value::Str(s.to_string())
    }
}

// SGR code for a named color/attribute, or None if `func` is not a style name.
// Foreground colors 30-37 + bright variants 90-97; attributes bold/dim/...
fn sgr_code(func: &str) -> Option<&'static str> {
    Some(match func {
        // foreground colors
        "black" => "30",
        "red" => "31",
        "green" => "32",
        "yellow" => "33",
        "blue" => "34",
        "magenta" => "35",
        "cyan" => "36",
        "white" => "37",
        "gray" | "grey" => "90", // bright black — the usual "muted" gray
        // attributes
        "bold" => "1",
        "dim" => "2",
        "italic" => "3",
        "underline" => "4",
        _ => return None,
    })
}

pub(crate) fn tui_module(func: &str, args: Vec<Value>) -> R {
    // --- styling: tui.<color>/<attr> s -> str (pure) ---
    if let Some(code) = sgr_code(func) {
        let s = arg_str(&args, 0, &format!("tui.{}", func))?;
        return Ok(style(code, &s));
    }

    match func {
        // tui.strip s -> str with all ANSI escapes removed. Useful to measure the
        // real width of styled text, or to log a clean copy.
        "strip" => {
            let s = arg_str(&args, 0, "tui.strip")?;
            Ok(Value::Str(strip_ansi(&s)))
        }

        // tui.print s -> write s to STDOUT with a trailing newline, no prefix.
        // `log` is for diagnostics (stderr, `[INFO]` prefix) — it corrupts a TUI.
        // tui.print is the clean channel for rendered widgets (box/table/badge).
        // tui.print with no arg prints a blank line.
        "print" => {
            let s = match args.first() {
                Some(v) => v.to_text(),
                None => String::new(),
            };
            let mut out = std::io::stdout();
            writeln!(out, "{}", s)
                .and_then(|_| out.flush())
                .map_err(|e| Flow::err(format!("tui.print: {}", e)))?;
            Ok(Value::Nil)
        }

        // --- text widgets (pure, -> str) ---

        // tui.rule  or  tui.rule "Title" -> a horizontal divider line spanning the
        // terminal width. With a title it reads `── Title ───────`.
        "rule" => {
            let title = match args.first() {
                Some(v) if !matches!(v, Value::Nil) => v.to_text(),
                _ => String::new(),
            };
            Ok(Value::Str(rule(&title)))
        }

        // tui.box "text"  or  tui.box "text" "Title" -> the text framed in a
        // rounded box. Multi-line text (\n) is boxed line by line.
        "box" => {
            let body = arg_str(&args, 0, "tui.box")?;
            let title = match args.get(1) {
                Some(v) if !matches!(v, Value::Nil) => v.to_text(),
                _ => String::new(),
            };
            Ok(Value::Str(boxed(&body, &title)))
        }

        // tui.badge "OK"  or  tui.badge "OK" :green -> a colored pill ` OK `.
        // Second arg is a color symbol/str (defaults to blue).
        "badge" => {
            let label = arg_str(&args, 0, "tui.badge")?;
            let color = match args.get(1) {
                Some(v) if !matches!(v, Value::Nil) => v.to_text(),
                _ => "blue".to_string(),
            };
            Ok(Value::Str(badge(&label, &color)))
        }

        // tui.table rows  or  tui.table rows headers -> an aligned text table.
        // `rows` is a list of lists (each inner list a row of cells). `headers`
        // (optional) is a list of column titles, shown bold with an underline rule.
        "table" => {
            let rows = match arg(&args, 0, "tui.table")? {
                Value::List(items) => items.clone(),
                other => {
                    return Err(Flow::err(format!(
                        "tui.table: rows must be a list of lists, got {}",
                        other.type_name()
                    )));
                }
            };
            // headers are optional, but if a second arg is GIVEN it must be a list —
            // a stray nil/str would otherwise silently render a header-less table and
            // hide the caller's mistake (the rest of tui.table surfaces shape errors).
            let headers = match args.get(1) {
                None | Some(Value::Nil) => None,
                Some(Value::List(h)) => Some(h.iter().map(|v| v.to_text()).collect::<Vec<_>>()),
                Some(other) => {
                    return Err(Flow::err(format!(
                        "tui.table: headers must be a list, got {}",
                        other.type_name()
                    )));
                }
            };
            table(&rows, headers.as_deref())
        }

        // --- interactive widgets (I/O) ---

        // tui.input "Name"  or  tui.input "Name" "default" -> a line of input.
        // Shows `Name: `, reads a line. Empty input falls back to the default
        // (when given). EOF (Ctrl-D) -> nil so a loop can stop.
        "input" => {
            let prompt = arg_str(&args, 0, "tui.input")?;
            let default = match args.get(1) {
                Some(v) if !matches!(v, Value::Nil) => Some(v.to_text()),
                _ => None,
            };
            input(&prompt, default.as_deref())
        }

        // tui.password "PIN" -> a line of input with the typed characters hidden.
        // Returns the typed string (or nil on EOF/Ctrl-C). Raw mode so nothing echoes.
        "password" => {
            let prompt = arg_str(&args, 0, "tui.password")?;
            password(&prompt)
        }

        // tui.confirm "Delete?"  or  tui.confirm "Delete?" true -> a yes/no prompt,
        // returns a bool. Second arg sets the default (when the user just presses
        // Enter); defaults to false. `[y/N]` / `[Y/n]` reflects the default.
        "confirm" => {
            let prompt = arg_str(&args, 0, "tui.confirm")?;
            let default = matches!(args.get(1), Some(Value::Bool(true)));
            confirm(&prompt, default)
        }

        // tui.select "Pick one" options -> arrow-key single choice. `options` is a
        // list of strings. Returns the chosen string (or nil if cancelled with
        // Esc/Ctrl-C). Up/Down (or j/k) to move, Enter to pick.
        "select" => {
            let prompt = arg_str(&args, 0, "tui.select")?;
            let options = str_list(&args, 1, "tui.select")?;
            select(&prompt, &options)
        }

        // tui.checkbox "Pick many" options -> arrow-key multi choice. Space toggles,
        // Enter confirms. Returns a list of the chosen strings (possibly empty), or
        // nil if cancelled.
        "checkbox" => {
            let prompt = arg_str(&args, 0, "tui.checkbox")?;
            let options = str_list(&args, 1, "tui.checkbox")?;
            checkbox(&prompt, &options)
        }

        _ => Err(Flow::err(format!(
            "tui module has no function '{}' (colors: green/red/yellow/blue/cyan/magenta/gray/white/bold/dim/italic/underline; widgets: rule/box/badge/table/input/password/confirm/select/checkbox/strip)",
            func
        ))),
    }
}

// A list-of-strings argument (for select/checkbox options).
fn str_list(args: &[Value], i: usize, who: &str) -> Result<Vec<String>, Flow> {
    match arg(args, i, who)? {
        Value::List(items) => Ok(items.iter().map(|v| v.to_text()).collect()),
        other => Err(Flow::err(format!(
            "{}: options must be a list, got {}",
            who,
            other.type_name()
        ))),
    }
}

// --- ANSI helpers ---

// Remove every `\x1b[...m` SGR sequence. Hand-rolled (no regex dep): scan for ESC,
// skip to the terminating `m`. Width-measuring code relies on this.
fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c == '\x1b' {
            // skip the `[` and everything up to and including the final letter
            for n in chars.by_ref() {
                if n.is_ascii_alphabetic() {
                    break;
                }
            }
        } else {
            out.push(c);
        }
    }
    out
}

// Visible width of a string, ignoring ANSI codes. char count is a good-enough proxy
// for terminal columns here (CLI labels are ASCII-heavy); wide CJK is not aligned
// perfectly but never corrupts layout.
fn vis_width(s: &str) -> usize {
    strip_ansi(s).chars().count()
}

// Terminal width, clamped to a sane default when it cannot be detected (piped output
// reports nothing). 80 is the universal fallback.
fn term_width() -> usize {
    terminal::size()
        .map(|(w, _)| w as usize)
        .unwrap_or(80)
        .max(8)
}

// --- design system (truecolor) ---
//
// The widgets share one palette so the whole battery looks like a single modern CLI
// (think Charm/gum, Clack, Vercel) rather than a pile of 16-color primitives. We use
// 24-bit truecolor — every terminal shipped in the last decade renders it; the named
// `tui.green`/etc. funcs stay on classic SGR for compatibility, but the *chrome*
// (frames, titles, pointers, badges) is rendered from these hex tokens.
//
// One signature element ties it together: a colored vertical bar `▌` down the left
// of every section heading and prompt — the thing you remember the look by.
//
// Tune the whole battery from here:
struct Rgb(u8, u8, u8);
const ACCENT: Rgb = Rgb(0x7c, 0x6f, 0xf0); // violet — the brand accent (bars, pointers)
const INK: Rgb = Rgb(0xe6, 0xe6, 0xf0); // near-white body text
const MUTED: Rgb = Rgb(0x6b, 0x6b, 0x80); // gray — frames, captions, hints
const OK: Rgb = Rgb(0x4a, 0xde, 0x80); // green — success
const WARN: Rgb = Rgb(0xff, 0xc1, 0x4a); // amber — warning
const DANGER: Rgb = Rgb(0xff, 0x6b, 0x6b); // red — error

// foreground / background truecolor SGR for a hex token
fn fg(c: &Rgb, s: &str) -> String {
    if color_enabled() {
        format!("\x1b[38;2;{};{};{}m{}{}", c.0, c.1, c.2, s, RESET)
    } else {
        s.to_string()
    }
}
fn on(bg: &Rgb, fgc: &Rgb, s: &str) -> String {
    if color_enabled() {
        format!(
            "\x1b[48;2;{};{};{};38;2;{};{};{};1m{}{}",
            bg.0, bg.1, bg.2, fgc.0, fgc.1, fgc.2, s, RESET
        )
    } else {
        s.to_string()
    }
}
fn bold(s: &str) -> String {
    if color_enabled() {
        format!("\x1b[1m{}{}", s, RESET)
    } else {
        s.to_string()
    }
}

// --- text widget rendering ---

// The signature: a violet vertical bar. A section heading is `▌ TITLE` — the title in
// bold ink, the bar in the accent. Quiet, modern, unmistakable.
const BAR: &str = "▌";

fn rule(title: &str) -> String {
    let width = term_width();
    if title.is_empty() {
        // a hairline the full width, in muted gray
        return fg(&MUTED, &"─".repeat(width));
    }
    // `▌ Title ──────` — accent bar, bold ink title, a trailing hairline.
    let used = 2 + vis_width(title) + 1; // bar + space + title + space
    let pad = width.saturating_sub(used + 1);
    format!(
        "{} {} {}",
        fg(&ACCENT, BAR),
        bold(title),
        fg(&MUTED, &"─".repeat(pad))
    )
}

fn boxed(body: &str, title: &str) -> String {
    let lines: Vec<&str> = body.split('\n').collect();
    let inner = lines
        .iter()
        .map(|l| vis_width(l))
        .max()
        .unwrap_or(0)
        .max(vis_width(title))
        + 2; // one space of breathing room on each side of the content
    // thin rounded frame in muted gray; the title rides on the top edge in ink
    let h = "─".repeat(inner);
    let bar = fg(&MUTED, "│");
    let mut out = String::new();
    if title.is_empty() {
        out.push_str(&fg(&MUTED, &format!("╭{}╮", h)));
        out.push('\n');
    } else {
        // `╭ Title ──────╮` — the top edge spans `inner` columns between the corners,
        // same as the body rows: ` ` + title + ` ` + dashes = inner, so the dash run
        // is inner - (title width) - 2 (the two flanking spaces). Off-by-one here used
        // to shift the top-right corner left and break the frame.
        let pad = inner.saturating_sub(vis_width(title) + 2);
        out.push_str(&fg(&MUTED, "╭ "));
        out.push_str(&bold(title));
        out.push(' ');
        out.push_str(&fg(&MUTED, &format!("{}╮", "─".repeat(pad))));
        out.push('\n');
    }
    for l in &lines {
        let pad = inner.saturating_sub(vis_width(l) + 2);
        out.push_str(&format!(
            "{} {}{} {}\n",
            bar,
            fg(&INK, l),
            " ".repeat(pad),
            bar
        ));
    }
    out.push_str(&fg(&MUTED, &format!("╰{}╯", h)));
    out
}

// Badge -> a filled pill in the status color. Modern look: solid background, bold
// label, a hair of horizontal padding. Off a tty it degrades to `[LABEL]`.
fn badge(label: &str, color: &str) -> String {
    // map the friendly color name to a palette token; default to the accent
    let bg = match color {
        "green" | "ok" => &OK,
        "yellow" | "warn" => &WARN,
        "red" | "danger" | "fail" => &DANGER,
        "gray" | "grey" | "muted" => &MUTED,
        _ => &ACCENT,
    };
    if color_enabled() {
        // dark ink ON the color reads cleanly on every status hue
        on(bg, &Rgb(0x10, 0x10, 0x18), &format!(" {} ", label))
    } else {
        format!("[{}]", label)
    }
}

fn table(rows: &[Value], headers: Option<&[String]>) -> R {
    // Flatten rows into Vec<Vec<String>>, validating shape.
    let mut grid: Vec<Vec<String>> = Vec::with_capacity(rows.len());
    for (ri, row) in rows.iter().enumerate() {
        match row {
            Value::List(cells) => grid.push(cells.iter().map(|c| c.to_text()).collect()),
            other => {
                return Err(Flow::err(format!(
                    "tui.table: row {} must be a list, got {}",
                    ri + 1,
                    other.type_name()
                )));
            }
        }
    }
    // Column count = widest row (or header count).
    let cols = grid
        .iter()
        .map(|r| r.len())
        .chain(headers.map(|h| h.len()))
        .max()
        .unwrap_or(0);
    if cols == 0 {
        return Ok(Value::Str(String::new()));
    }
    // Per-column width = widest visible cell (headers included).
    let mut widths = vec![0usize; cols];
    if let Some(h) = headers {
        for (c, cell) in h.iter().enumerate() {
            widths[c] = widths[c].max(vis_width(cell));
        }
    }
    for row in &grid {
        for (c, cell) in row.iter().enumerate() {
            widths[c] = widths[c].max(vis_width(cell));
        }
    }
    let pad_cell = |cell: &str, w: usize| -> String {
        let extra = w.saturating_sub(vis_width(cell));
        format!("{}{}", cell, " ".repeat(extra))
    };
    let gap = "   "; // generous 3-space gutters — airy, modern
    let mut out = String::new();
    if let Some(h) = headers {
        // headers: violet accent, UPPERCASE — a clear column label band
        let line: Vec<String> = (0..cols)
            .map(|c| {
                let title = h.get(c).map(String::as_str).unwrap_or("").to_uppercase();
                fg(&ACCENT, &pad_cell(&title, widths[c]))
            })
            .collect();
        out.push_str(&line.join(gap));
        out.push('\n');
        // a hairline under the header band, in muted gray
        let rule: Vec<String> = widths.iter().map(|w| "─".repeat(*w)).collect();
        out.push_str(&fg(&MUTED, &rule.join(gap)));
        out.push('\n');
    }
    for (ri, row) in grid.iter().enumerate() {
        let line: Vec<String> = (0..cols)
            .map(|c| {
                fg(
                    &INK,
                    &pad_cell(row.get(c).map(String::as_str).unwrap_or(""), widths[c]),
                )
            })
            .collect();
        out.push_str(line.join(gap).trim_end());
        if ri + 1 < grid.len() {
            out.push('\n');
        }
    }
    Ok(Value::Str(out))
}

// --- interactive widgets ---

// Print a prompt to stdout (no newline) and flush so it shows before input.
fn show(prompt: &str) -> Result<(), Flow> {
    let mut out = std::io::stdout();
    out.write_all(prompt.as_bytes())
        .and_then(|_| out.flush())
        .map_err(|e| Flow::err(format!("tui: {}", e)))
}

// The prompt line shared by input/confirm/password: a violet accent bar, the
// question in ink, an optional dim hint. `▌ Your name (anon) › `
fn prompt_line(prompt: &str, hint: Option<&str>) -> String {
    let mut s = format!("{} {}", fg(&ACCENT, BAR), fg(&INK, prompt.trim_start()));
    if let Some(h) = hint {
        s.push(' ');
        s.push_str(&fg(&MUTED, h));
    }
    s.push(' ');
    s.push_str(&fg(&MUTED, "›"));
    s.push(' ');
    s
}

fn input(prompt: &str, default: Option<&str>) -> R {
    let hint = default
        .filter(|d| !d.is_empty())
        .map(|d| format!("({})", d));
    show(&prompt_line(prompt, hint.as_deref()))?;
    let mut line = String::new();
    match std::io::stdin().read_line(&mut line) {
        Ok(0) => Ok(Value::Nil), // EOF
        Ok(_) => {
            let s = line.trim_end_matches(['\n', '\r']);
            if s.is_empty() {
                match default {
                    Some(d) => Ok(Value::Str(d.to_string())),
                    None => Ok(Value::Str(String::new())),
                }
            } else {
                Ok(Value::Str(s.to_string()))
            }
        }
        Err(e) => Err(Flow::err(format!("tui.input: {}", e))),
    }
}

fn confirm(prompt: &str, default: bool) -> R {
    let hint = if default { "(Y/n)" } else { "(y/N)" };
    show(&prompt_line(prompt, Some(hint)))?;
    let mut line = String::new();
    match std::io::stdin().read_line(&mut line) {
        Ok(0) => Ok(Value::Bool(default)),
        Ok(_) => {
            let s = line.trim().to_lowercase();
            let yes = match s.as_str() {
                "" => default,
                "y" | "yes" => true,
                _ => false,
            };
            Ok(Value::Bool(yes))
        }
        Err(e) => Err(Flow::err(format!("tui.confirm: {}", e))),
    }
}

// An interactive widget needs a real terminal on BOTH ends: stdin to drive keys, and
// stdout because every prompt/redraw is written there. If either is redirected
// (`fluxon run wizard.fx > out.txt`) the user sees nothing and control bytes leak
// into the file. Call this BEFORE printing any prompt so nothing leaks on failure.
fn require_tty() -> Result<(), Flow> {
    if !std::io::stdin().is_terminal() || !std::io::stdout().is_terminal() {
        return Err(Flow::err(
            "tui: this widget needs an interactive terminal (stdin and stdout must both be a tty)",
        ));
    }
    Ok(())
}

// A RAII guard that turns raw mode off on drop — so any early return / `?` / panic
// still restores the terminal. Entering twice is harmless; crossterm tracks state.
struct RawGuard;
impl RawGuard {
    fn enter() -> Result<Self, Flow> {
        require_tty()?;
        terminal::enable_raw_mode().map_err(|e| Flow::err(format!("tui: raw mode: {}", e)))?;
        Ok(RawGuard)
    }
}
impl Drop for RawGuard {
    fn drop(&mut self) {
        let _ = terminal::disable_raw_mode();
    }
}

// Read one key event in raw mode, mapped to a small action enum. Filters out key
// *release* events (Windows reports both press and release).
enum Key {
    Up,
    Down,
    Enter,
    Space,
    Cancel, // Esc or Ctrl-C
    Other,  // any other key — ignored by the navigation widgets
}

// Map a key event to a navigation action for select/checkbox. The vim (j/k) and
// space mappings live HERE, local to navigation — password reads raw events itself
// so those characters are never stolen from a typed secret.
fn read_key() -> Result<Key, Flow> {
    loop {
        match event::read().map_err(|e| Flow::err(format!("tui: read key: {}", e)))? {
            Event::Key(k) if k.kind != KeyEventKind::Release => {
                if k.modifiers.contains(KeyModifiers::CONTROL)
                    && matches!(k.code, KeyCode::Char('c'))
                {
                    return Ok(Key::Cancel);
                }
                return Ok(match k.code {
                    KeyCode::Up | KeyCode::Char('k') => Key::Up,
                    KeyCode::Down | KeyCode::Char('j') => Key::Down,
                    KeyCode::Enter => Key::Enter,
                    KeyCode::Char(' ') => Key::Space,
                    KeyCode::Esc => Key::Cancel,
                    _ => Key::Other,
                });
            }
            _ => continue,
        }
    }
}

fn password(prompt: &str) -> R {
    // Check the tty BEFORE printing, so a redirected run leaks nothing.
    require_tty()?;
    show(&prompt_line(prompt, None))?;
    let _guard = RawGuard::enter()?;
    let mut buf = String::new();
    // Password reads RAW keys directly — NOT through read_key(), whose vim/space
    // mappings (j->Down, k->Up, ' '->Space) would silently swallow those very chars
    // from the secret. Here every printable char counts, including j/k/space.
    loop {
        match event::read().map_err(|e| Flow::err(format!("tui: read key: {}", e)))? {
            Event::Key(k) if k.kind != KeyEventKind::Release => {
                let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
                match k.code {
                    KeyCode::Enter => break,
                    KeyCode::Esc => return password_cancel(),
                    KeyCode::Char('c') if ctrl => return password_cancel(),
                    KeyCode::Backspace => {
                        if buf.pop().is_some() {
                            // erase one dot: back up, overwrite with space, back up again
                            print!("\x08 \x08");
                            let _ = std::io::stdout().flush();
                        }
                    }
                    KeyCode::Char(c) => {
                        buf.push(c);
                        // echo a dot per char so the user sees progress, length hidden
                        print!("{}", fg(&MUTED, "•"));
                        let _ = std::io::stdout().flush();
                    }
                    _ => {}
                }
            }
            _ => continue,
        }
    }
    print!("\r\n");
    let _ = std::io::stdout().flush();
    Ok(Value::Str(buf))
}

// Cancel a password prompt: drop a newline so the next output isn't glued on, return nil.
fn password_cancel() -> R {
    print!("\r\n");
    let _ = std::io::stdout().flush();
    Ok(Value::Nil)
}

// Draw the option list for select/checkbox. Each row is rewritten in place by moving
// the cursor up `count` lines first (after the initial draw).
fn redraw(options: &[String], cursor: usize, checked: Option<&[bool]>, first: bool) {
    let mut out = std::io::stdout();
    if !first {
        // move cursor up to the first option line to overwrite it
        let _ = write!(out, "\x1b[{}A", options.len());
    }
    for (i, opt) in options.iter().enumerate() {
        let active = i == cursor;
        // violet ▌ bar marks the active row (the signature element, reused here)
        let pointer = if active {
            fg(&ACCENT, BAR)
        } else {
            " ".to_string()
        };
        // Every row carries a marker so the cursor is unmistakable:
        //   checkbox -> ◉ green ticked / ◯ muted unticked (multi-select)
        //   select   -> ● violet on the active row / ◯ muted elsewhere (radio; one pick)
        let mark = match checked {
            Some(c) if c[i] => fg(&OK, "◉ "),
            Some(_) => fg(&MUTED, "◯ "),
            None if active => fg(&ACCENT, "● "),
            None => fg(&MUTED, "◯ "),
        };
        // active label in bold ink, others muted — focus reads instantly
        let label = if active {
            bold(&fg(&INK, opt))
        } else {
            fg(&MUTED, opt)
        };
        // clear the line then print; \r\n because raw mode does not translate \n
        let _ = write!(out, "\x1b[2K{} {}{}\r\n", pointer, mark, label);
    }
    let _ = out.flush();
}

fn select(prompt: &str, options: &[String]) -> R {
    if options.is_empty() {
        return Err(Flow::err("tui.select: options list is empty"));
    }
    require_tty()?; // before any print — a redirected run must leak nothing
    println!(
        "{} {}  {}",
        fg(&ACCENT, BAR),
        fg(&INK, prompt.trim_start()),
        fg(&MUTED, "↑↓ move · ⏎ select")
    );
    let _guard = RawGuard::enter()?;
    let mut cursor = 0usize;
    let mut first = true;
    let result = loop {
        redraw(options, cursor, None, first);
        first = false;
        match read_key()? {
            Key::Up => {
                cursor = if cursor == 0 {
                    options.len() - 1
                } else {
                    cursor - 1
                }
            }
            Key::Down => cursor = (cursor + 1) % options.len(),
            Key::Enter => break Some(options[cursor].clone()),
            Key::Cancel => break None,
            _ => {}
        }
    };
    drop(_guard);
    println!(); // settle the cursor below the list
    match result {
        Some(s) => Ok(Value::Str(s)),
        None => Ok(Value::Nil),
    }
}

fn checkbox(prompt: &str, options: &[String]) -> R {
    if options.is_empty() {
        return Err(Flow::err("tui.checkbox: options list is empty"));
    }
    require_tty()?; // before any print — a redirected run must leak nothing
    println!(
        "{} {}  {}",
        fg(&ACCENT, BAR),
        fg(&INK, prompt.trim_start()),
        fg(&MUTED, "space toggle · ⏎ confirm")
    );
    let _guard = RawGuard::enter()?;
    let mut cursor = 0usize;
    let mut checked = vec![false; options.len()];
    let mut first = true;
    let confirmed = loop {
        redraw(options, cursor, Some(&checked), first);
        first = false;
        match read_key()? {
            Key::Up => {
                cursor = if cursor == 0 {
                    options.len() - 1
                } else {
                    cursor - 1
                }
            }
            Key::Down => cursor = (cursor + 1) % options.len(),
            Key::Space => checked[cursor] = !checked[cursor],
            Key::Enter => break true,
            Key::Cancel => break false,
            _ => {}
        }
    };
    drop(_guard);
    println!();
    if !confirmed {
        return Ok(Value::Nil);
    }
    let chosen: Vec<Value> = options
        .iter()
        .zip(checked.iter())
        .filter(|&(_, &c)| c)
        .map(|(o, _)| Value::Str(o.clone()))
        .collect();
    Ok(Value::List(chosen))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::builtins::is_module;

    fn s(x: &str) -> Value {
        Value::Str(x.to_string())
    }

    #[test]
    fn tui_is_module() {
        assert!(is_module("tui"));
    }

    // tui.print returns nil (writing to stdout is a side effect). It prints "" here
    // so the test output is not polluted. No-arg form is allowed (blank line).
    #[test]
    fn print_returns_nil() {
        match tui_module("print", vec![s("")]) {
            Ok(Value::Nil) => {}
            _ => panic!("tui.print must return nil"),
        }
        match tui_module("print", vec![]) {
            Ok(Value::Nil) => {}
            _ => panic!("tui.print with no arg must return nil"),
        }
    }

    // Color funcs are pure str -> str. We can't force a tty in the test runner, so
    // assert the no-tty branch: the text is returned unchanged (no escapes).
    #[test]
    fn color_passthrough_without_tty() {
        match tui_module("green", vec![s("hi")]) {
            Ok(Value::Str(out)) => assert_eq!(out, "hi"),
            _ => panic!("tui.green must return a str"),
        }
    }

    #[test]
    fn unknown_func_errors() {
        match tui_module("nope", vec![]) {
            Err(Flow::Error(msg)) => assert!(msg.contains("tui module")),
            _ => panic!("expected Flow::Error"),
        }
    }

    // strip_ansi removes SGR codes and leaves text intact.
    #[test]
    fn strip_removes_escapes() {
        let styled = "\x1b[32mhi\x1b[0m there";
        assert_eq!(strip_ansi(styled), "hi there");
        match tui_module("strip", vec![s(styled)]) {
            Ok(Value::Str(out)) => assert_eq!(out, "hi there"),
            _ => panic!("tui.strip must return a str"),
        }
    }

    // vis_width ignores escapes (used by table/box alignment).
    #[test]
    fn vis_width_ignores_escapes() {
        assert_eq!(vis_width("\x1b[1mABC\x1b[0m"), 3);
        assert_eq!(vis_width("héllo"), 5);
    }

    // A table aligns columns to the widest cell and shows an UPPERCASE header band.
    #[test]
    fn table_aligns_columns() {
        let rows = Value::List(vec![
            Value::List(vec![s("a"), s("100")]),
            Value::List(vec![s("bb"), s("2")]),
        ]);
        let headers = Value::List(vec![s("name"), s("n")]);
        match tui_module("table", vec![rows, headers]) {
            Ok(Value::Str(out)) => {
                // headers are rendered uppercase ("name" -> "NAME")
                assert!(out.contains("NAME"));
                assert!(out.contains("bb"));
                // header band + hairline rule + 2 data rows
                assert!(out.lines().count() >= 4);
            }
            _ => panic!("tui.table must return a str"),
        }
    }

    // table rejects a non-list row with a clear error.
    #[test]
    fn table_rejects_bad_row() {
        let rows = Value::List(vec![s("not a list")]);
        assert!(tui_module("table", vec![rows]).is_err());
    }

    // A given-but-non-list headers arg is an explicit error (not silently dropped).
    // nil headers stay valid (= "no headers").
    #[test]
    fn table_rejects_bad_headers() {
        let rows = Value::List(vec![Value::List(vec![s("a")])]);
        assert!(tui_module("table", vec![rows.clone(), s("oops")]).is_err());
        assert!(tui_module("table", vec![rows.clone(), Value::Int(1)]).is_err());
        // nil -> treated as no headers, ok
        assert!(tui_module("table", vec![rows, Value::Nil]).is_ok());
    }

    // box frames single and multi-line bodies; output has the corner glyphs.
    #[test]
    fn box_frames_body() {
        match tui_module("box", vec![s("hello\nworld")]) {
            Ok(Value::Str(out)) => {
                assert!(out.starts_with('╭'));
                assert!(out.contains("hello"));
                assert!(out.contains("world"));
                assert!(out.ends_with('╯'));
            }
            _ => panic!("tui.box must return a str"),
        }
    }

    // A titled box stays a rectangle: every line (top edge, body rows, bottom edge)
    // has the same visible width — guards the off-by-one that shifted the top-right
    // corner when the body was wider than the title.
    #[test]
    fn box_titled_stays_aligned() {
        match tui_module("box", vec![s("Build passed"), s("Status")]) {
            Ok(Value::Str(out)) => {
                let widths: Vec<usize> = out.lines().map(vis_width).collect();
                assert!(
                    widths.windows(2).all(|w| w[0] == w[1]),
                    "box rows misaligned: {:?}",
                    widths
                );
            }
            _ => panic!("tui.box must return a str"),
        }
    }

    // badge without a tty falls back to [LABEL] (no escapes).
    #[test]
    fn badge_plain_without_tty() {
        match tui_module("badge", vec![s("OK")]) {
            Ok(Value::Str(out)) => assert_eq!(out, "[OK]"),
            _ => panic!("tui.badge must return a str"),
        }
    }

    // rule without a title is a full line of the box-drawing dash.
    #[test]
    fn rule_is_a_line() {
        match tui_module("rule", vec![]) {
            Ok(Value::Str(out)) => assert!(out.chars().all(|c| c == '─')),
            _ => panic!("tui.rule must return a str"),
        }
    }

    // select/checkbox require a non-empty options list.
    #[test]
    fn select_empty_options_errors() {
        assert!(tui_module("select", vec![s("pick"), Value::List(vec![])]).is_err());
        assert!(tui_module("checkbox", vec![s("pick"), Value::List(vec![])]).is_err());
    }

    // require_tty must reject when stdout is not a terminal — the guard that keeps a
    // redirected run (`fluxon run wizard.fx > out.txt`) from leaking prompts. Under
    // `cargo test` stdout is piped, so this exercises the rejecting path. (We test
    // the guard directly rather than the widgets, so a tty-attached runner can't
    // make password() block waiting on a keypress.)
    #[test]
    fn require_tty_rejects_non_tty() {
        if !std::io::stdout().is_terminal() {
            assert!(require_tty().is_err());
        }
    }
}

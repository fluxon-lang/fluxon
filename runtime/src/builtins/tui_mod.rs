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

        // tui.md s -> str: render a Markdown string for the terminal (the subset AI
        // batteries actually emit — headings, bold/italic, inline+fenced code, lists,
        // quotes, links, rules). Reuses the same palette as the rest of the battery,
        // so AI output looks like the assistant, not a raw pager. Pure (-> str), the
        // caller decides where it goes — `tui.print (tui.md (ai.ask "…"))`.
        "md" => {
            let s = arg_str(&args, 0, "tui.md")?;
            Ok(Value::Str(render_markdown(&s)))
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

        // tui.badge "OK"  or  tui.badge "OK" :green -> a filled pill ` OK `.
        // Second arg is a status color (green/ok, yellow/warn, red/danger/fail,
        // gray/muted) or the default violet accent; any other name uses the accent.
        "badge" => {
            let label = arg_str(&args, 0, "tui.badge")?;
            let color = match args.get(1) {
                Some(v) if !matches!(v, Value::Nil) => v.to_text(),
                _ => "accent".to_string(),
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
            "tui module has no function '{}' (colors: green/red/yellow/blue/cyan/magenta/gray/white/bold/dim/italic/underline; widgets: rule/box/badge/table/md/input/password/confirm/select/checkbox/strip)",
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
// Bold + a truecolor foreground in ONE SGR sequence — avoids nesting bold(fg(..)),
// whose inner reset would otherwise close the bold mid-string.
fn fg_bold(c: &Rgb, s: &str) -> String {
    if color_enabled() {
        format!("\x1b[1;38;2;{};{};{}m{}{}", c.0, c.1, c.2, s, RESET)
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
    // `▌ Title ──────` — accent bar, bold ink title, a trailing hairline that fills
    // the line to `width`. Visible columns: bar(1) + space(1) + title + space(1) +
    // dashes, so dashes = width - title - 3. (The format string adds the two spaces.)
    let used = 1 + vis_width(title) + 1 + 1; // bar + space + title + space
    let pad = width.saturating_sub(used);
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
// Extra badge hues so `:blue`/`:cyan`/`:magenta` (valid in tui.<color>) also work as
// a badge background, instead of silently falling back to the accent.
const BLUE: Rgb = Rgb(0x4a, 0x90, 0xff);
const CYAN: Rgb = Rgb(0x3a, 0xd0, 0xd8);
const MAGENTA: Rgb = Rgb(0xd6, 0x6f, 0xe0);

fn badge(label: &str, color: &str) -> String {
    // map the friendly color name to a palette token; unknown -> the accent
    let bg = match color {
        "green" | "ok" => &OK,
        "yellow" | "warn" => &WARN,
        "red" | "danger" | "fail" => &DANGER,
        "gray" | "grey" | "muted" => &MUTED,
        "blue" => &BLUE,
        "cyan" => &CYAN,
        "magenta" => &MAGENTA,
        _ => &ACCENT, // "accent" and any unrecognized name
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
        // NOTE: do NOT trim_end() the joined row — a sparse row (fewer cells than
        // columns) is padded with spaces for the missing columns, and trimming would
        // collapse them and break alignment. Every row keeps its full column width.
        out.push_str(&line.join(gap));
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
        // EOF (stdin closed/redirected) is NOT the same as pressing Enter — an
        // unattended run must not silently "accept" a default-yes prompt and trigger
        // a destructive action. Return nil (falsy in Fluxon) so the caller can detect
        // "no answer" and a bare `if (tui.confirm ...)` does NOT proceed. Matches
        // tui.input / io.read_line, which also return nil on EOF.
        Ok(0) => Ok(Value::Nil),
        Ok(_) => {
            let s = line.trim().to_lowercase();
            let yes = match s.as_str() {
                "" => default, // a bare Enter applies the default
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
            fg_bold(&INK, opt)
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

// --- markdown rendering ---
//
// A tight, hand-rolled renderer for the Markdown subset AI batteries emit. We do NOT
// pull a CommonMark crate on purpose: the subset is small and fixed, and we want full
// control over how each element maps onto the battery's palette (so AI output looks
// like the rest of `tui`, not a bolted-on theme).
//
// The pipeline is two layers, mirroring Markdown's own block/inline split:
//
//   parse_blocks(src) -> Vec<Block>   — group lines into block elements (heading,
//                                        paragraph, code fence, list, quote, rule).
//   render_block(&Block) -> String    — render one block, calling render_inline for
//                                        any text that can carry `**`/`*`/`code`/links.
//
// The block layer is deliberately the seam for a future STREAMING renderer (#198 v2):
// a streaming caller buffers incoming chunks, splits off the COMPLETE blocks (every
// block but the last, which may still be growing — an open ``` fence, an unfinished
// list), renders and prints those, and keeps the tail in the buffer. Keeping all
// multi-line state (fence open/closed, list nesting) inside parse_blocks — not smeared
// across a line-at-a-time loop — is what makes that buffering tractable later.

// One Markdown block element. Inline markup inside text fields is resolved at render
// time by render_inline, not here.
enum Block {
    // `# H` .. `### H` — level is 1..=3 (deeper headings render as level 3).
    Heading { level: u8, text: String },
    // a run of consecutive non-blank text lines, joined — inline markup applies.
    Paragraph(Vec<String>),
    // a ``` fenced block: raw lines (NO inline markup — code is literal) + optional lang.
    Code { lang: String, lines: Vec<String> },
    // a list: each item is (marker, indent depth, text). Ordered items carry their
    // number in `marker` ("1."), unordered a bullet ("-"). Nesting is by indent.
    List(Vec<ListItem>),
    // `> quote` — the quoted lines (markers stripped); inline markup applies.
    Quote(Vec<String>),
    // `---` / `***` / `___` — a horizontal rule.
    Rule,
}

struct ListItem {
    depth: usize,  // indent level (0 = top), one level per ~2 leading spaces
    ordered: bool, // true for `1.`, false for `-`/`*`/`+`
    number: usize, // the item's number when ordered (else unused)
    text: String,  // the item text — inline markup applies
}

// Split Markdown source into blocks. Walks lines, tracking the multi-line state that
// a line-at-a-time renderer would get wrong: an open code fence swallows everything
// (including blank lines and `#`) until its closing fence.
fn parse_blocks(src: &str) -> Vec<Block> {
    let mut blocks = Vec::new();
    let lines: Vec<&str> = src.lines().collect();
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i];
        let trimmed = line.trim_start();

        // fenced code: ``` or ~~~ (optionally with a language). Everything up to the
        // matching closing fence is literal — no inline markup, no other block parsing.
        if let Some(fence) = code_fence(trimmed) {
            // The opener's marker char + length define the close: a ``` block is NOT
            // closed by a `~~~` line, nor by a SHORTER run — only a same-char run of at
            // least the opener's length, with nothing after it (CommonMark). Without
            // this, a `~~~` (or longer ```) appearing inside the code would falsely end
            // the block and the rest of the literal body would be re-parsed as Markdown.
            let marker = fence.chars().next().unwrap();
            let open_len = fence.len();
            let lang = trimmed[open_len..].trim().to_string();
            i += 1;
            let mut code = Vec::new();
            while i < lines.len() && !closes_fence(lines[i].trim_start(), marker, open_len) {
                code.push(lines[i].to_string());
                i += 1;
            }
            i += 1; // consume the closing fence (or run off the end — unterminated is ok)
            blocks.push(Block::Code { lang, lines: code });
            continue;
        }

        // blank line — a block separator, nothing to emit.
        if trimmed.is_empty() {
            i += 1;
            continue;
        }

        // horizontal rule — a line of only -, * or _ (3+), possibly spaced.
        if is_hr(trimmed) {
            blocks.push(Block::Rule);
            i += 1;
            continue;
        }

        // heading — 1..6 leading `#` then a space. Levels deeper than 3 clamp to 3.
        if let Some((level, text)) = heading(trimmed) {
            blocks.push(Block::Heading {
                level,
                text: text.to_string(),
            });
            i += 1;
            continue;
        }

        // blockquote — consecutive `>` lines, markers stripped.
        if trimmed.starts_with('>') {
            let mut quoted = Vec::new();
            while i < lines.len() && lines[i].trim_start().starts_with('>') {
                let q = lines[i].trim_start();
                // strip the leading `>` and one optional following space
                let rest = q[1..].strip_prefix(' ').unwrap_or(&q[1..]);
                quoted.push(rest.to_string());
                i += 1;
            }
            blocks.push(Block::Quote(quoted));
            continue;
        }

        // list — consecutive lines that each start with a bullet or `N.` marker.
        if list_marker(line).is_some() {
            let mut items = Vec::new();
            while i < lines.len() {
                match list_marker(lines[i]) {
                    Some((ordered, number, depth, text)) => {
                        items.push(ListItem {
                            depth,
                            ordered,
                            number,
                            text: text.to_string(),
                        });
                        i += 1;
                    }
                    None => break,
                }
            }
            blocks.push(Block::List(items));
            continue;
        }

        // otherwise a paragraph — gather consecutive "plain" lines until a blank line
        // or the start of another block kind.
        let mut para = Vec::new();
        while i < lines.len() {
            let t = lines[i].trim_start();
            if t.is_empty()
                || code_fence(t).is_some()
                || is_hr(t)
                || heading(t).is_some()
                || t.starts_with('>')
                || list_marker(lines[i]).is_some()
            {
                break;
            }
            para.push(lines[i].trim().to_string());
            i += 1;
        }
        blocks.push(Block::Paragraph(para));
    }
    blocks
}

// Render a parsed block list into a terminal string. Blocks are separated by a BLANK
// line (`\n\n`), matching how Markdown reads on the page — a paragraph followed by a
// list/heading must not sit flush against it.
fn render_markdown(src: &str) -> String {
    let blocks = parse_blocks(src);
    let parts: Vec<String> = blocks.iter().map(render_block).collect();
    parts.join("\n\n")
}

fn render_block(b: &Block) -> String {
    match b {
        Block::Heading { level, text } => render_heading(*level, text),
        Block::Paragraph(lines) => {
            let joined = lines.join(" ");
            render_inline(&joined)
        }
        Block::Code { lang, lines } => render_code(lang, lines),
        Block::List(items) => render_list(items),
        Block::Quote(lines) => render_quote(lines),
        Block::Rule => rule(""),
    }
}

// Heading: the signature accent bar + bold text. H1 also gets an underline rule beneath
// (it's the document title); H2/H3 step down to plain accent, dim accent — a clear but
// quiet hierarchy that reuses ACCENT/MUTED rather than inventing new colors.
fn render_heading(level: u8, text: &str) -> String {
    let inline = render_inline(text);
    match level {
        1 => {
            let head = format!("{} {}", fg(&ACCENT, BAR), fg_bold(&ACCENT, &strip_md(text)));
            // underline the title's visible width with a hairline
            let w = vis_width(&head);
            format!("{}\n{}", head, fg(&MUTED, &"─".repeat(w)))
        }
        2 => format!("{} {}", fg(&ACCENT, BAR), fg_bold(&INK, &strip_md(text))),
        _ => format!("{} {}", fg(&MUTED, BAR), bold(&inline)),
    }
}

// Inline markup pass over one line of text. Handles, in a single left-to-right scan:
//   `code`            -> distinct color, never re-parsed (literal)
//   **bold** / __b__  -> bold SGR
//   *italic* / _i_    -> italic SGR
//   [text](url)       -> underlined text + dim url
// Backtick code spans win over emphasis (so `**` inside `code` stays literal), matching
// CommonMark precedence for the common cases agents emit.
fn render_inline(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        match c {
            // inline code span — copy verbatim between the backticks, styled.
            '`' => {
                if let Some(end) = find_char(&chars, i + 1, '`') {
                    let code: String = chars[i + 1..end].iter().collect();
                    out.push_str(&code_span(&code));
                    i = end + 1;
                    continue;
                }
                out.push(c);
                i += 1;
            }
            // link [text](url)
            '[' => {
                if let Some(link) = parse_link(&chars, i) {
                    out.push_str(&render_link(&link.text, &link.url));
                    i = link.end;
                    continue;
                }
                out.push(c);
                i += 1;
            }
            // bold (** or __) — needs the doubled marker.
            '*' | '_' if i + 1 < chars.len() && chars[i + 1] == c => {
                // `_` emphasis does NOT open inside a word (CommonMark intraword rule),
                // so `created_at`/`foo__bar` keep their underscores literal; `*` may
                // open anywhere. The close marker must likewise not sit inside a word.
                if can_open(&chars, i, c)
                    && let Some(end) = find_run(&chars, i + 2, c, 2)
                    && can_close(&chars, end, 2, c)
                {
                    let inner: String = chars[i + 2..end].iter().collect();
                    out.push_str(&bold(&render_inline(&inner)));
                    i = end + 2;
                    continue;
                }
                out.push(c);
                i += 1;
            }
            // italic (* or _) — single marker.
            '*' | '_' => {
                if can_open(&chars, i, c)
                    && let Some(end) = find_char(&chars, i + 1, c)
                    && can_close(&chars, end, 1, c)
                {
                    let inner: String = chars[i + 1..end].iter().collect();
                    out.push_str(&italic(&inner));
                    i = end + 1;
                    continue;
                }
                out.push(c);
                i += 1;
            }
            _ => {
                out.push(c);
                i += 1;
            }
        }
    }
    out
}

struct Link {
    text: String,
    url: String,
    end: usize, // index just past the closing `)`
}

// Parse a `[text](url)` link starting at `[`. Returns None if the shape doesn't match
// (a lone `[` stays literal).
fn parse_link(chars: &[char], start: usize) -> Option<Link> {
    let close_text = find_char(chars, start + 1, ']')?;
    if chars.get(close_text + 1) != Some(&'(') {
        return None;
    }
    let close_url = find_char(chars, close_text + 2, ')')?;
    Some(Link {
        text: chars[start + 1..close_text].iter().collect(),
        url: chars[close_text + 2..close_url].iter().collect(),
        end: close_url + 1,
    })
}

// underlined link text + a dim url in parens. (OSC 8 hyperlinks are a tempting upgrade
// but render as junk on terminals that don't support them; the dim-url form is safe
// everywhere and still shows the destination.)
fn render_link(text: &str, url: &str) -> String {
    // The link label can itself carry markup (`[**docs**](url)`), so render it through
    // the inline pass in BOTH branches — off a tty that strips the markers to clean
    // text (the no-tty promise), on a tty it styles them. Only the underline wrapper
    // and dim url are color-gated.
    let label = render_inline(text);
    if color_enabled() {
        format!(
            "\x1b[4m{}{} {}",
            label,
            RESET,
            fg(&MUTED, &format!("({})", url))
        )
    } else {
        format!("{} ({})", label, url)
    }
}

// inline `code` — a distinct color so it reads as code without a heavy background
// (which fights with the body text on most themes). Cyan reads as "literal/technical".
fn code_span(s: &str) -> String {
    fg(&CYAN, s)
}

fn italic(s: &str) -> String {
    if color_enabled() {
        format!("\x1b[3m{}{}", s, RESET)
    } else {
        s.to_string()
    }
}

// A fenced code block: each line indented under a muted frame bar, with an optional
// dim language label on top. The bar (reused signature element) sets it apart from the
// prose without a full box, and survives copy-paste better than a background fill.
fn render_code(lang: &str, lines: &[String]) -> String {
    let bar = fg(&MUTED, "│");
    let mut out = String::new();
    if !lang.is_empty() {
        out.push_str(&fg(&MUTED, &format!("{} {}", "│", lang)));
        out.push('\n');
    }
    for (i, l) in lines.iter().enumerate() {
        // code is literal — no inline markup; INK keeps it readable, the bar marks it.
        out.push_str(&format!("{} {}", bar, fg(&INK, l)));
        if i + 1 < lines.len() {
            out.push('\n');
        }
    }
    out
}

// A list: one line per item, indented by depth, marked with a violet bullet (unordered)
// or its dim number (ordered). Inline markup applies to the item text.
fn render_list(items: &[ListItem]) -> String {
    let mut out = String::new();
    for (i, it) in items.iter().enumerate() {
        let indent = "  ".repeat(it.depth);
        let marker = if it.ordered {
            fg(&MUTED, &format!("{}.", it.number))
        } else {
            fg(&ACCENT, "•")
        };
        out.push_str(&format!("{}{} {}", indent, marker, render_inline(&it.text)));
        if i + 1 < items.len() {
            out.push('\n');
        }
    }
    out
}

// A blockquote: the signature accent bar down the left, dim text — the "reuses the
// signature element" note from the issue. Each quoted line keeps inline markup.
fn render_quote(lines: &[String]) -> String {
    let bar = fg(&ACCENT, BAR);
    let mut out = String::new();
    for (i, l) in lines.iter().enumerate() {
        out.push_str(&format!("{} {}", bar, fg(&MUTED, &render_inline(l))));
        if i + 1 < lines.len() {
            out.push('\n');
        }
    }
    out
}

// --- markdown parse helpers ---

// If `s` OPENS a code fence (``` or ~~~, 3+ of the same char), return the fence run so
// the caller can read the trailing language. Else None.
fn code_fence(s: &str) -> Option<&str> {
    for marker in ['`', '~'] {
        let run: String = s.chars().take_while(|&c| c == marker).collect();
        if run.len() >= 3 {
            // return the fence slice (its byte length equals the run length, ASCII).
            return Some(&s[..run.len()]);
        }
    }
    None
}

// Does `s` CLOSE a fence opened with `marker` x `open_len`? A close is a run of the
// SAME marker char, at least as long as the opener, and nothing else after it (only
// trailing spaces). This is stricter than code_fence: a ``` block ignores a `~~~` line
// and a shorter ``` run, so neither falsely terminates the literal body.
fn closes_fence(s: &str, marker: char, open_len: usize) -> bool {
    let run = s.chars().take_while(|&c| c == marker).count();
    run >= open_len && s[run..].trim().is_empty()
}

// A horizontal rule: 3+ of -, * or _ and nothing else but spaces.
fn is_hr(s: &str) -> bool {
    let stripped: String = s.chars().filter(|c| !c.is_whitespace()).collect();
    stripped.len() >= 3
        && (stripped.chars().all(|c| c == '-')
            || stripped.chars().all(|c| c == '*')
            || stripped.chars().all(|c| c == '_'))
}

// `# Heading` -> (level, text). Level is the count of leading `#` (1..=6, clamped to 3
// at render), then at least one space.
fn heading(s: &str) -> Option<(u8, &str)> {
    let hashes = s.chars().take_while(|&c| c == '#').count();
    if (1..=6).contains(&hashes) && s.as_bytes().get(hashes) == Some(&b' ') {
        let level = hashes.min(3) as u8;
        Some((level, s[hashes + 1..].trim()))
    } else {
        None
    }
}

// A list item marker at the START of `line` (leading spaces set the depth). Returns
// (ordered, number, depth, text). `- `/`* `/`+ ` are unordered; `N. `/`N) ` ordered.
fn list_marker(line: &str) -> Option<(bool, usize, usize, &str)> {
    let indent = line.len() - line.trim_start().len();
    let depth = indent / 2; // ~2 spaces per nesting level (the common agent style)
    let rest = line.trim_start();
    // unordered: a bullet then a space
    for b in ['-', '*', '+'] {
        if let Some(after) = rest.strip_prefix(b)
            && after.starts_with(' ')
        {
            return Some((false, 0, depth, after.trim_start()));
        }
    }
    // ordered: digits, then `.` or `)`, then a space
    let digits = rest.chars().take_while(|c| c.is_ascii_digit()).count();
    if digits > 0 {
        let after = &rest[digits..];
        if (after.starts_with(". ") || after.starts_with(") "))
            && let Ok(n) = rest[..digits].parse()
        {
            return Some((true, n, depth, after[2..].trim_start()));
        }
    }
    None
}

// Markdown's intraword-underscore rule, simplified to what agents emit: a `_` run may
// open emphasis only at a word boundary, and close only at one. A `*` run has no such
// restriction — `foo*bar*baz` is valid emphasis — so `*` always passes. This keeps
// `created_at`, `foo_bar_baz`, `a__b__c` literal in plain prose.
//
// "word boundary" = the char just OUTSIDE the marker run is not part of a word. We
// treat both alphanumerics AND other `_` markers as word material: in `a__b__c`, the
// `_` flanking `b` is glued to surrounding underscores, not a true boundary, so no span
// opens or closes. (`is_word_edge` returns true at the string edge or a real separator.)
fn is_word_edge(c: Option<&char>) -> bool {
    match c {
        None => true,
        Some(c) => !c.is_alphanumeric() && *c != '_',
    }
}
fn can_open(chars: &[char], idx: usize, marker: char) -> bool {
    if marker != '_' {
        return true;
    }
    is_word_edge(idx.checked_sub(1).and_then(|p| chars.get(p)))
}
fn can_close(chars: &[char], end: usize, run_len: usize, marker: char) -> bool {
    if marker != '_' {
        return true;
    }
    // `end` is the closing run's first marker char; the char just PAST the whole run
    // (end + run_len) must be a word edge — so `a_b_c` doesn't italicize `b`.
    is_word_edge(chars.get(end + run_len))
}

// Find the next index >= from where chars[idx] == target. None if absent.
fn find_char(chars: &[char], from: usize, target: char) -> Option<usize> {
    (from..chars.len()).find(|&i| chars[i] == target)
}

// Find the start of the next run of `n` consecutive `target` chars at/after `from`.
// Used to close a `**bold**` span (n=2).
fn find_run(chars: &[char], from: usize, target: char, n: usize) -> Option<usize> {
    let mut i = from;
    while i + n <= chars.len() {
        if chars[i..i + n].iter().all(|&c| c == target) {
            return Some(i);
        }
        i += 1;
    }
    None
}

// Strip Markdown inline markers from text, leaving the words — used where a heading is
// re-styled wholesale (H1/H2 are already bold+colored, so nested `**`/`*` would just
// add stray markers). Removes `*`, `_`, backticks; keeps link text, drops the url.
fn strip_md(s: &str) -> String {
    strip_ansi(&render_inline(s))
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

    // A titled rule fills the line to the same width as a plain rule — guards the
    // off-by-one that made `tui.rule "X"` one column short.
    #[test]
    fn rule_titled_fills_width() {
        let plain = match tui_module("rule", vec![]) {
            Ok(Value::Str(s)) => vis_width(&s),
            _ => panic!("tui.rule must return a str"),
        };
        let titled = match tui_module("rule", vec![s("Title")]) {
            Ok(Value::Str(s)) => vis_width(&s),
            _ => panic!("tui.rule must return a str"),
        };
        assert_eq!(plain, titled, "titled rule must fill to the same width");
    }

    // A sparse table row (fewer cells than columns) keeps full width — guards the
    // trim_end() that used to collapse the missing columns and break alignment.
    #[test]
    fn table_sparse_row_stays_aligned() {
        let rows = Value::List(vec![
            Value::List(vec![s("aaa"), s("bbb"), s("ccc")]),
            Value::List(vec![s("x")]), // short row
        ]);
        match tui_module(
            "table",
            vec![rows, Value::List(vec![s("c1"), s("c2"), s("c3")])],
        ) {
            Ok(Value::Str(out)) => {
                let widths: Vec<usize> = out.lines().map(vis_width).collect();
                assert!(
                    widths.windows(2).all(|w| w[0] == w[1]),
                    "table rows misaligned: {:?}",
                    widths
                );
            }
            _ => panic!("tui.table must return a str"),
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

    // confirm must return nil on EOF, NOT the default — an unattended/redirected run
    // must not silently auto-accept a default-yes prompt. Only run when stdin is not
    // a tty (cargo test, CI), so read_line hits immediate EOF instead of blocking.
    #[test]
    fn confirm_eof_is_nil_not_default() {
        if !std::io::stdin().is_terminal() {
            // even with default=true, EOF must yield nil (never auto-yes)
            match confirm("Ship?", true) {
                Ok(Value::Nil) => {}
                _ => panic!("confirm must return nil on EOF, never the default"),
            }
        }
    }

    // --- markdown ---

    // Helper: render via the public dispatch and return the string (panics otherwise).
    fn md(src: &str) -> String {
        match tui_module("md", vec![s(src)]) {
            Ok(Value::Str(out)) => out,
            _ => panic!("tui.md must return a str"),
        }
    }

    // Under `cargo test` stdout is piped, so color is OFF — tui.md returns CLEAN text
    // (no ANSI escapes, no literal `**`/`#`/backticks). This is the contract that makes
    // the output safe for files and grep, and it lets us assert on the plain text.
    #[test]
    fn md_plain_without_tty() {
        let out = md("# Title\n\nSome **bold** and *italic* and `code` here.");
        assert!(!out.contains('\x1b'), "no escapes off a tty: {:?}", out);
        // markup markers are consumed, only the words remain
        assert!(out.contains("Title"));
        assert!(out.contains("bold"));
        assert!(out.contains("italic"));
        assert!(out.contains("code"));
        assert!(
            !out.contains("**"),
            "bold markers must be stripped: {:?}",
            out
        );
        assert!(
            !out.contains('#'),
            "heading marker must be stripped: {:?}",
            out
        );
        assert!(
            !out.contains('`'),
            "code markers must be stripped: {:?}",
            out
        );
    }

    // A heading renders its text (and H1 gets an underline rule line beneath it).
    #[test]
    fn md_heading_h1_underlined() {
        let out = md("# Hello");
        let lines: Vec<&str> = out.lines().collect();
        assert!(lines[0].contains("Hello"));
        // second line is the hairline underline (box-drawing dashes)
        assert!(lines.len() >= 2 && lines[1].chars().all(|c| c == '─'));
    }

    // A fenced code block keeps its lines LITERAL — `**` inside code is not bolded
    // away, and the content survives verbatim.
    #[test]
    fn md_code_fence_is_literal() {
        let out = md("```rust\nlet x = **y;\n```");
        assert!(
            out.contains("let x = **y;"),
            "code must stay literal: {:?}",
            out
        );
    }

    // An unterminated fence (no closing ```) still renders its body — agents stream
    // partial output, and a missing close must not drop the code.
    #[test]
    fn md_unterminated_fence_renders() {
        let out = md("```\nhello world");
        assert!(out.contains("hello world"));
    }

    // Ordered and unordered lists render each item; nested items indent.
    #[test]
    fn md_lists_render_items() {
        let out = md("- one\n- two\n  - nested");
        assert!(out.contains("one") && out.contains("two") && out.contains("nested"));
        // the nested item is indented relative to a top-level one
        let nested = out.lines().find(|l| l.contains("nested")).unwrap();
        assert!(
            nested.starts_with("  "),
            "nested item must indent: {:?}",
            nested
        );

        let ordered = md("1. first\n2. second");
        assert!(ordered.contains("first") && ordered.contains("second"));
        // ordered items keep their numbers
        assert!(ordered.contains("1.") && ordered.contains("2."));
    }

    // A link keeps both the text and the url (so the destination is never lost).
    #[test]
    fn md_link_keeps_text_and_url() {
        let out = md("see [the docs](https://example.com) now");
        assert!(out.contains("the docs"));
        assert!(out.contains("https://example.com"));
    }

    // Markup INSIDE link text is rendered, not left literal — off a tty the markers
    // are stripped (the no-tty clean-text promise applies to link labels too).
    #[test]
    fn md_link_text_markup_is_stripped_off_tty() {
        let out = md("see [**the docs**](https://example.com)");
        assert!(out.contains("the docs"));
        assert!(
            !out.contains("**"),
            "link label markers must be stripped off a tty: {:?}",
            out
        );
        let code_label = md("read [`config`](https://x.io)");
        assert!(code_label.contains("config"));
        assert!(
            !code_label.contains('`'),
            "code-span markers in a link label must be stripped: {:?}",
            code_label
        );
    }

    // Blocks are separated by a BLANK line — a paragraph followed by a list/heading
    // must not render flush against it. Guards the `\n` (vs `\n\n`) join.
    #[test]
    fn md_blocks_separated_by_blank_line() {
        let out = md("a paragraph\n\n- item one\n- item two");
        // exactly one empty line sits between the paragraph and the first list item
        let lines: Vec<&str> = out.lines().collect();
        let para = lines.iter().position(|l| l.contains("paragraph")).unwrap();
        let first_item = lines.iter().position(|l| l.contains("item one")).unwrap();
        assert!(
            first_item == para + 2 && lines[para + 1].is_empty(),
            "a blank line must separate the blocks: {:?}",
            lines
        );
    }

    // A blockquote keeps its text (marker stripped).
    #[test]
    fn md_quote_strips_marker() {
        let out = md("> a wise note");
        assert!(out.contains("a wise note"));
        assert!(
            !out.contains('>'),
            "quote marker must be stripped: {:?}",
            out
        );
    }

    // `---` becomes a rule (a full line of the box-drawing dash).
    #[test]
    fn md_thematic_break_is_rule() {
        let out = md("above\n\n---\n\nbelow");
        assert!(
            out.lines()
                .any(|l| !l.is_empty() && l.chars().all(|c| c == '─'))
        );
    }

    // The block parser — the streaming seam — groups lines into the right block kinds
    // and keeps a code fence as ONE block (not split at its blank/`#` inner lines).
    #[test]
    fn md_parse_blocks_groups_correctly() {
        let blocks =
            parse_blocks("# H\n\npara line\n\n```\n# not a heading\n\ncode\n```\n\n- item");
        // heading, paragraph, code, list — exactly four blocks
        assert_eq!(blocks.len(), 4);
        assert!(matches!(blocks[0], Block::Heading { level: 1, .. }));
        assert!(matches!(blocks[1], Block::Paragraph(_)));
        match &blocks[2] {
            Block::Code { lines, .. } => {
                // the `#` and blank line inside the fence are CODE, not new blocks
                assert!(lines.iter().any(|l| l.contains("# not a heading")));
            }
            _ => panic!("third block must be a code fence"),
        }
        assert!(matches!(blocks[3], Block::List(_)));
    }

    // A bare paragraph with no markup passes through unchanged.
    #[test]
    fn md_plain_paragraph_passthrough() {
        let out = md("just some words");
        assert_eq!(out, "just some words");
    }

    // A snake_case identifier in plain prose keeps its underscores — `_` emphasis must
    // not open/close inside a word (CommonMark intraword rule). Guards the bug where
    // `created_at` rendered as `createdat` and `foo_bar_baz` as `foobarbaz`.
    #[test]
    fn md_intraword_underscores_preserved() {
        assert_eq!(md("the created_at column"), "the created_at column");
        assert_eq!(md("call foo_bar_baz here"), "call foo_bar_baz here");
        assert_eq!(md("a__b__c stays"), "a__b__c stays");
        // but a real `_italic_` at a word boundary still works (text survives, marker gone)
        let out = md("an _emphasized_ word");
        assert!(out.contains("emphasized") && !out.contains('_'));
        // `*` emphasis is allowed intraword (CommonMark) — text survives, markers gone
        let star = md("foo*bar*baz");
        assert!(star.contains("bar") && !star.contains('*'));
    }

    // A code fence is closed ONLY by a run of the SAME marker char, at least as long as
    // the opener. Guards the bug where a `~~~` (or longer ```) line inside a ``` block
    // falsely ended it and the rest leaked back into Markdown parsing.
    #[test]
    fn md_fence_close_matches_opener() {
        // a ~~~ line inside a ``` block is just code, not the close
        let out = md("```\nline one\n~~~\nline two\n```");
        assert!(out.contains("line one") && out.contains("line two"));
        assert!(
            out.contains("~~~"),
            "the inner ~~~ must stay literal: {:?}",
            out
        );
        // the whole thing is ONE code block, so the inner lines are never re-parsed
        let blocks = parse_blocks("```\n# not a heading\n~~~\n```");
        assert_eq!(blocks.len(), 1);
        match &blocks[0] {
            Block::Code { lines, .. } => {
                assert!(lines.iter().any(|l| l.contains("# not a heading")));
                assert!(lines.iter().any(|l| l.contains("~~~")));
            }
            _ => panic!("must be a single code block"),
        }
    }

    // A shorter ``` run does not close a longer ```` opener (CommonMark length rule).
    #[test]
    fn md_fence_close_needs_open_length() {
        let blocks = parse_blocks("````\n```\nstill code\n````");
        assert_eq!(blocks.len(), 1);
        match &blocks[0] {
            Block::Code { lines, .. } => {
                assert!(lines.iter().any(|l| l.contains("```")));
                assert!(lines.iter().any(|l| l.contains("still code")));
            }
            _ => panic!("must be a single code block"),
        }
    }
}

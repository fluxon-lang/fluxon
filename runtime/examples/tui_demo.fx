# tui battery — terminal UI for CLI tools and agents.
# Use `tui.print` (clean stdout), NOT `log` — log adds an `[INFO]` prefix on stderr
# and would corrupt the layout. Run in a real terminal to see the colors.

# --- colors & styles (str -> str) ---
tui.print (tui.green "✓ ok") + "  " + (tui.red "✗ fail") + "  " + (tui.dim "muted")
tui.print (tui.bold "bold") + " " + (tui.underline "underline") + " " + (tui.cyan "cyan")

# --- a divider with a title ---
tui.print (tui.rule "Report")

# --- a framed box ---
tui.print (tui.box "Build passed\n12 tests, 0 failures" "Status")

# --- badges ---
tui.print (tui.badge "PASS" :green) + " " + (tui.badge "WARN" :yellow) + " " + (tui.badge "FAIL" :red)

# --- an aligned table with headers ---
rows = [
  ["alice" "admin"  "active"]
  ["bob"   "editor" "invited"]
  ["carol" "viewer" "active"]
]
tui.print (tui.table rows ["user" "role" "status"])

# --- interactive (uncomment to try in a real terminal) ---
# name = tui.input "Your name" "anon"
# ok   = tui.confirm "Continue?" true
# pick = tui.select "Pick an env" ["dev" "staging" "prod"]
# tags = tui.checkbox "Labels" ["bug" "docs" "perf"]
# tui.print "name=${name} ok=${ok} env=${pick} tags=${tags}"

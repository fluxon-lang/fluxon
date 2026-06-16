# tui showcase — every component, end to end.
# Run it in a REAL terminal so the truecolor palette and arrow keys work:
#   cargo run -- run examples/tui_showcase.fx
#
# Print widgets with `tui.print` (clean stdout), NOT `log` (stderr + `[INFO]`).
# IMPORTANT: a paren-free call grabs the rest of the line, so wrap a whole
# `a + b + c` expression in parens: tui.print (x + y), not tui.print x + y.

# a small helper: a section heading (the ▌ accent bar comes from tui.rule)
fn sec title
  tui.print ""
  tui.print (tui.rule title)

# ---------- 1. colors & styles ----------
sec "Colors & styles"
tui.print ("  " + (tui.green "green") + "   " + (tui.red "red") + "   " + (tui.yellow "yellow") + "   " + (tui.blue "blue"))
tui.print ("  " + (tui.cyan "cyan") + "   " + (tui.magenta "magenta") + "   " + (tui.gray "gray"))
tui.print ("  " + (tui.bold "bold") + "   " + (tui.dim "dim") + "   " + (tui.italic "italic") + "   " + (tui.underline "underline"))

# ---------- 2. box ----------
sec "Box"
tui.print (tui.box "Build passed\n12 tests · 0 failures\nready to ship" "Status")

# ---------- 3. badges ----------
sec "Badges"
tui.print ("  " + (tui.badge "PASS" :green) + "  " + (tui.badge "WARN" :yellow) + "  " + (tui.badge "FAIL" :red) + "  " + (tui.badge "NEW" :accent))

# ---------- 4. table ----------
sec "Table"
users = [
  ["alice" "admin"  "active"]
  ["bob"   "editor" "invited"]
  ["carol" "viewer" "active"]
]
tui.print (tui.table users ["user" "role" "status"])

# ---------- 5. input ----------
sec "Input"
name = tui.input "Your name" "anon"
tui.print ("  → hello, " + (tui.green name))

# ---------- 6. select ----------
sec "Select"
env = tui.select "Deploy to" ["dev" "staging" "prod"]
if env == nil
  env <- "dev"
tui.print ("  → environment: " + (tui.cyan env))

# ---------- 7. checkbox ----------
sec "Checkbox"
steps = tui.checkbox "Pre-deploy steps" ["run tests" "build assets" "tag release" "notify team"]
if steps == nil
  steps <- []
tui.print ("  → selected: " + (tui.cyan "${steps}"))

# ---------- 8. confirm ----------
sec "Confirm"
go = tui.confirm "Ship to ${env}?" true
if go
  tui.print ("  " + (tui.badge "DEPLOYING" :green) + " to " + (tui.bold env))
else
  tui.print ("  " + (tui.badge "ABORTED" :red))

# ---------- 9. password ----------
sec "Password"
secret = tui.password "Enter a token (or Enter to skip)"
if secret == nil
  tui.print (tui.dim "  skipped")
else
  tui.print ("  → received " + (tui.green "${str.len secret} chars"))

tui.print ""
tui.print (tui.rule)
tui.print ("  " + (tui.green "✓") + " " + (tui.bold "showcase complete"))
tui.print ""

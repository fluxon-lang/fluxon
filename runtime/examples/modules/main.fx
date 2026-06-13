# User modules — `use ./file` (issue #45).
# The path is resolved relative to this file's directory; `.fx` extension is auto.

use ./greet                 # greet.fx -> greet.* namespace

log (greet.hello "Aziza")   # Hello, Aziza!
log "lang: ${greet.lang}"   # lang: english

# Renaming with `as` (avoids clashing with a battery name).
use ./greet as g
log (g.hello "Bobur")       # Hello, Bobur!

# A module-private name (`prefix`) is not in the namespace — returns nil.
(greet.prefix == nil) | (fail "a private name must not be exported")
log "ok: private name is hidden"

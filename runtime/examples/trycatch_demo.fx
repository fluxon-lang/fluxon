# try/catch demo (issue #125) — catch an error and keep going.
# Run: cargo run -- run examples/trycatch_demo.fx

# 1) Fallback: on error, continue with a default value.
fn get_price product
  if product == "none"
    fail 404 "product not found: ${product}"
  ret 100

price = try
  get_price "none"
catch e
  log "warning: ${e.message} (status: ${e.status})"
  0                                  # fallback price
log "price = ${price}"               # -> 0

# 2) Custom error: raise a fail from your own business rule.
fn validate items
  if (items.len) != 4
    fail "the submitted data must contain exactly 4 items"
  ret :ok

msg = try
  validate [1 2 3]
catch e
  e.message
log msg                              # -> the submitted data must contain exactly 4...

# 3) Take the first one that works from several sources (with re-raise).
fn primary -> fail "primary source failed"
fn fallback -> "data from fallback"

result = try
  primary()
catch e
  log "primary failed: ${e.message} — switching to fallback"
  try
    fallback()
  catch e2
    fail "both sources failed: ${e2.message}"
log result                           # -> data from fallback

# 4) Retry: on error, try a few times.
attempt <- 0
fn flaky
  attempt <- attempt + 1
  if attempt < 3
    fail "temporary error (attempt ${attempt})"
  ret "success"

reply <- nil
each i in 1..3
  reply <- try
    flaky()
  catch e
    log "attempt failed: ${e.message}"
    nil
  if reply != nil
    stop
log "final reply: ${reply}"          # -> success (on the 3rd attempt)

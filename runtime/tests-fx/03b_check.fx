# 03b-check - verifies the row count in a second process after rollback.
# The previous process tried to add a "ghost" row via tx+fail.
# If rollback works correctly -> only the "original" remains (len == 1).
use db

tbl items
  id   serial pk
  name str

rows = db.q "select * from items"
if rows.len == 1
  log "=== 03b_rollback: ROLLBACK-OK (row count 1, ghost not added) ==="
else
  log "=== 03b_rollback: FAIL - row count ${rows.len} (rollback did not work) ==="

# 03b-check — rollback'dan keyin ikkinchi jarayonda qator sonini tasdiqlaydi.
# Avvalgi jarayon "ghost" qatorini tx+fail bilan qo'shmoqchi bo'lgan.
# Rollback to'g'ri ishlasa → faqat "asl" qoladi (len == 1).
use db

tbl items
  id   serial pk
  name str

rows = db.q "select * from items"
if rows.len == 1
  log "=== 03b_rollback: ROLLBACK-OK (qator soni 1, ghost qo'shilmadi) ==="
else
  log "=== 03b_rollback: FAIL — qator soni ${rows.len} (rollback ishlamadi) ==="

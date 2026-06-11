# cron_jobs.fluxon — Cron ishlar: soatlik faollik logi va boshqalar

use db cron
use ./realtime

# Soatlik: faol kanallar va xabar hajmini log qilish
fn log_active_channels
  log "=== Soatlik kanal statistikasi ==="

  # So'nggi 1 soatdagi faol kanallar
  active = db.q "select c.id, c.name,
                        count(m.id) as msg_count,
                        max(m.created) as last_msg
                 from channels c
                 left join messages m on m.channel = c.id
                   and m.created > $1
                   and m.is_blocked = false
                 group by c.id, c.name
                 having count(m.id) > 0
                 order by msg_count desc" [(time.ago 1 :hr)]

  if active.len == 0
    log "So'nggi 1 soatda hech qanday faollik yo'q"
    ret nil

  log "Faol kanallar soni: ${active.len}"
  each row in active
    log "  [${row.name}] xabarlar: ${row.msg_count} | so'nggi: ${row.last_msg}"

  # Jami statistika
  total = db.one "select count(*) c from messages where created > $1 and is_blocked=false" [(time.ago 1 :hr)]
  log "Jami so'nggi 1 soatda: ${total.c ?? 0} xabar"

  # Online foydalanuvchilar
  online = db.one "select count(*) c from users where status=$1" ["online"]
  log "Online foydalanuvchilar: ${online.c ?? 0}"
  log "==================================="

# Kunlik: o'chirilishi kerak bo'lgan eski flaglangan xabarlarni tekshirish
fn daily_moderation_review
  log "=== Kunlik moderatsiya tekshiruvi ==="

  flagged = db.q "select m.id, m.body, m.created, u.username, c.name as channel_name
                  from messages m
                  join users u on u.id = m.user
                  join channels c on c.id = m.channel
                  where m.is_flagged = true
                    and m.created > $1
                  order by m.created desc" [(time.ago 24 :hr)]

  log "So'nggi 24 soatda flaglangan xabarlar: ${flagged.len}"
  each msg in flagged
    log "  [${msg.channel_name}] @${msg.username}: ${str.slice msg.body 0 80}"

  log "====================================="

# Haftalik: nofaol foydalanuvchilarni offline qilish
fn weekly_cleanup
  log "=== Haftalik tozalash ==="

  # 7 kun faol bo'lmagan online ko'rinib turganlarni offline qil
  # SPEC GAP: db.up WHERE IN yoki subquery qo'llab-quvvatlanmaydi,
  # shuning uchun har birini alohida update qilish kerak
  stale = db.q "select u.id from users u
                where u.status = $1
                and u.id not in (
                  select distinct m.user from messages m
                  where m.created > $2
                )" ["online" (time.ago 7 :day)]

  each user in stale
    db.up "users" {status::offline} {id:user.id}

  log "Offline qilingan nofaol foydalanuvchilar: ${stale.len}"
  log "========================="

# Soatlik: xabar hajmini yozib borish (analytics)
fn record_hourly_stats
  now_ts   = time.now
  msg_cnt  = db.one "select count(*) c from messages where created > $1" [(time.ago 1 :hr)]
  user_cnt = db.one "select count(*) c from users where status=$1" ["online"]

  db.ins "hourly_stats" {
    ts:           now_ts
    message_count: msg_cnt.c ?? 0
    online_users: user_cnt.c ?? 0
  }
  log "Soatlik stat yozildi: xabar=${msg_cnt.c ?? 0} online=${user_cnt.c ?? 0}"

# Analytics jadvali
tbl hourly_stats
  id             serial pk
  ts             now
  message_count  int
  online_users   int

# ─── Cron jadvali ────────────────────────────────────────────────────────────

# Soatlik, har soat 0 daqiqasida: faol kanallar logi
cron.hr 0 log_active_channels

# Soatlik, har soat 30 daqiqasida: statistika yozish
cron.hr 30 record_hourly_stats

# Kunlik, soat 03:00 da: moderatsiya tekshiruvi
cron.dy 3 0 daily_moderation_review

# Haftalik, yakshanba soat 02:00 da: tozalash
cron.wk :sun 2 0 weekly_cleanup

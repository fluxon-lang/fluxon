# schema.flux — Barcha tbl ta'riflari
# Ushbu fayl faqat schema ta'riflari. Boshqa fayllar use ./schema orqali import qiladi.

use db

# Foydalanuvchilar jadvali
tbl users
  id       serial pk
  username str uniq
  email    str uniq
  status   sym        # :online :offline :away
  created  now

# Kanallar jadvali
tbl channels
  id         serial pk
  name       str uniq
  is_private bool
  created_by int ref:users.id
  created    now

# Kanal a'zoligi jadvali
tbl memberships
  id         serial pk
  channel    int ref:channels.id
  user       int ref:users.id
  role       sym        # :owner :admin :member
  joined     now

# Xabarlar jadvali
tbl messages
  id         serial pk
  channel    int ref:channels.id
  user       int ref:users.id
  body       str
  is_blocked bool
  is_flagged bool
  created    now

# Reaksiyalar jadvali
tbl reactions
  id         serial pk
  message    int ref:messages.id
  user       int ref:users.id
  emoji      str

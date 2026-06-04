# Validatsiya yordamchilari

# Oddiy email tekshiruvi: '@' va '.' bo'lishi, bo'sh bo'lmasligi kerak
exp fn valid_email s
  if s == nil
    ret false
  if str.len s == 0
    ret false
  if !(str.has s "@")
    ret false
  ret str.has s "."

# Matn bo'sh emasligini tekshiradi (nil yoki bo'sh satr emas)
exp fn non_empty s
  if s == nil
    ret false
  ret str.len s > 0

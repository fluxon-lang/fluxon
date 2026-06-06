# 12 testi uchun yordamchi modul. Faqat `exp` qilingan nomlar tashqaridan ko'rinadi.

# Modul-private — namespace'ga kirmaydi.
base = 100

exp pi = 3

exp fn add a b -> a + b

# Closure: modul-darajadagi `base`ga kiradi (import qiluvchi scope'iga emas).
exp fn from_base n -> base + n

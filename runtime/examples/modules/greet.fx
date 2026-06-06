# greet — kichik modul. Faqat `exp` qilingan nomlar tashqaridan ko'rinadi.

# Modul-private (eksport qilinmagan): tashqaridan ko'rinmaydi.
prefix = "Salom"

# Eksport qilingan qiymat.
exp lang = "o'zbekcha"

# Eksport qilingan funksiya — modul-darajadagi `prefix`ga (closure) kira oladi.
exp fn hello nom -> "${prefix}, ${nom}!"

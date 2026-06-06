# 12 testi uchun: o'zi boshqa modulni import qiladigan modul (nested import).
use ./mod_math

# mod_math.add'ni qayta ishlatadi — nested import shu modulning katalogiga
# nisbatan hal qilinadi.
exp fn double n -> mod_math.add n n

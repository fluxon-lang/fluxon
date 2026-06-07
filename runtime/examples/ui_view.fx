# Flux Frontend — 1-2 BOSQICH misoli: view + theme + each/if -> HTML.
#
# `view` = komponent (fn'ning UI varianti). `theme` = global dizayn tokenlari
# (CSS custom properties). `each`/`if`/`match` view ichida ro'yxat/shartli render.
# Reaktivlik/server/source — keyingi bosqichlar.

# Global dizayn tokenlari (CSS custom properties'ga aylanadi).
theme
  primary "#e84d8a"
  radius  :lg
  muted   "#888"

# Oddiy komponent: ko'p elementli tana fragmentga yig'iladi.
view greeting name
  h1 "Salom $name"
  p "xush kelibsiz" {kind::muted}

# each (ro'yxat) + if (shartli) view ichida.
view menu items
  h2 "Menyu"
  each it in items
    if it == "Atirgul"
      p it {kind::primary}
    else
      p it

# Element bolalari indentatsiya orqali; semantik proplar -> CSS class.
view card title price
  div {kind::panel}
    h2 title
    p "${price} so'm" {kind::muted}
    btn "Sotib olish" {kind::primary}

log (ui.html (greeting "Ali"))
log (ui.html (menu ["Atirgul" "Lola" "Chinnigul"]))
log (ui.html (card "Atirgul" 50000))

# To'liq HTML hujjat (doctype + theme CSS + body).
log (ui.page (card "Lola" 35000))

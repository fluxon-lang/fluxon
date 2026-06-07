# Frontend dizayni — Flux'ga UI qatlami qanday qo'shildi

Bu papka Flux **frontend qatlamining** dizayn jarayonini saqlaydi. Backend tili
`language-design/` da tug'ilgani kabi, frontend ham taxmin bilan emas, AI
modellarini stress-test qilish orqali qurildi.

Maqsad: AI kam kod yozsin, lekin UI **moslashuvchan** bo'lsin — shadcn/ui kabi
tayyor bloklar (DEFAULT) → tema/config → o'z view'ini yozish (OVERRIDE).

Har round Workflow orqali: 3 model (opus/sonnet/haiku) mustaqil ishlaydi +
1 opus tahlilchi konvergensiyani/bo'shliqlarni topadi. Har papkada `agent-a`
(opus), `agent-b` (sonnet), `agent-c` (haiku) va `analysis/`.

## `round1-invented-ui-langs/`
Modellarga **Flux berilmadi** — har biri frontend uchun yangi UI tilini o'zi
ixtiro qilib gul do'koni dashboard yozdi. **Natija:** 3 model mustaqil deyarli
bir xil arxitekturaga keldi, 2 tasi tilni "Petal" deb atadi. Komponent =
parametrli blok, avtomatik reaktivlik, `each`/`if`, scoped stil, default
komponentlar — konvergent. `analysis/OLD-flux-frontend-rejected.md` — eski
(React/`{tag::}` map) yondashuv, keyingi roundlarda RAD etilgan.

## `round2-flux-integration/`
Modellar **Flux docs'ini ko'rib**, unga organik mos frontend dizayn qildi.
**Eng muhim natija:** uchala model ham yangi reaktiv belgi ixtiro QILMADI —
Flux'ning mavjud `<-` (mutable bind) ni reaktiv state deb oldi (boshqa tillar
`~`/useState ixtiro qilishga majbur — Flux ustun). Opus 3654 token bilan to'liq
5-sahifa dashboard yozdi. Bu round'dan `docs/flux-frontend.md` v1 tug'ildi:
5 primitiv (`view`/`theme`/`page`/`source`/`act`), `ui.*` batareya.

## `round3-stress-test/`
v1 spec'ni **ko'rmagan** modellarga restoran admin paneli (yangi soha) yozdirildi.
Har model topgan "bo'shliq" tilning haqiqiy kamchiligini ko'rsatdi. **9 bo'shliq
topildi va v2'da yopildi** — eng kritiklari: frontend WS (realtime "yolg'on
va'da" → `source live` + `ui.push`/`ui.on`), `fn`→`view` state scope (→ `act`),
source tag qoidasi, dinamik source. `analysis/gaps.md` har bo'shliq + spec
tuzatishini batafsil yozadi. (`_workflow.js` — workflow ish fayli, commit emas.)

> Eksperiment artefaktlari — ishlab chiqarish kodi emas. Ba'zi panel kodlarida
> ataylab/taxminiy xato bor (modellar topgan bo'shliqlar) — ular spec qanday
> yaxshilanganini ko'rsatadi. Yakuniy spec: `docs/flux-frontend.md`.

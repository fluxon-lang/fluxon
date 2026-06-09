# Round 6 — 2-bosqich validatsiya (builder implementatsiyadan KEYIN)

1-bosqichda dizayn tanlandi (builder), keyin `db_mod.rs`/`lexer.rs`/`interp.rs`
ga **haqiqatan implementatsiya qilindi**. Bu bosqich: haiku agent yangilangan
(ishlaydigan) spec bilan booking+analytics PRD'ni yozadi va kod **haqiqiy
`flux` binary'da run** qilinadi (1-bosqich faqat statik o'qigan edi).

## Natija

| Ko'rsatkich | 1-bosqich (xom SQL spec) | 2-bosqich (builder spec) |
|-------------|:--:|:--:|
| Builder chaqiruvi (`db.from \|>`) | 0 (yo'q edi) | **11** |
| Xom SQL (`db.q`) | har o'qish/analitika | **3** (faqat JOIN'lar) |
| `/bookings` filtr+IN+range+order+paging | qo'lda `$N` string | **toza builder** |
| `/stats/overview` (status sanoq+revenue) | 6 round-trip yoki butun jadval | **count_if/sum_if/agg_row** |
| `str.sym` (status→sym) | `json.dec` hack | **`(str.split q ",").map \s -> str.sym s`** |

Agent xom SQL'ni **faqat** 3 ta haqiqiy JOIN endpoint'da ishlatdi
(`/stats/by-resource`, `/stats/daily`, `/stats/top-customers`) — bular builder
qoplay olmaydigan ko'p-jadvalli/`date()` so'rovlari, ya'ni to'g'ri qaror. Qolgan
butun CRUD + overview deklarativ builder bilan, **bironta qo'lda SQL string'siz**.

## Haqiqiy run

Kod `DATABASE_URL=sqlite::memory: flux run` bilan tekshirildi (server'siz, faqat
yuklash/eval). **Bitta haqiqiy xato** bor edi:

- **Ko'p-qatorli string literal** (302-qator): agent JOIN SQL'ni ko'p qatorga
  yoyib yozdi, lekin Flux string'lari bir qatorda bo'lishi kerak. Bu builder
  qismida EMAS — escape-hatch'dagi string formatlashda.

O'sha xom-SQL string'ларни bir qatorga siqib (boshqa hech narsa o'zgartirmasdan)
— **butun backend toza yuklandi** (`exit 0`, xatosiz): 5 `tbl`, JWT middleware,
10 endpoint, 11 builder zanjiri, 3 JOIN.

> Eslatma: workflow ichidagi haiku "run-check" agenti `ok:true` deb noto'g'ri
> xabar berdi (run natijasini noto'g'ri talqin qildi). Haqiqiy holatni asosiy
> agent o'zi `flux` binary'da tasdiqladi — sub-agent run-natijasiga ishonib
> bo'lmaydi.

## Topilmalar → keyingi ish

1. **Builder ishladi va agent uni to'g'ri ishlatdi** — issue #78 maqsadiga
   erishildi: o'qishlar deklarativ, IN/range/order/paging xom SQL'siz.
2. **Ko'p-qatorli string yo'qligi** agentni chalg'itadi (JOIN SQL'da). Spec'da
   xom SQL misolini doim BIR qatorda ko'rsatish kerak (allaqachon shunday), lekin
   agent baribir yoydi → kelajakda til ko'p-qatorli string qo'llashi mumkin
   (alohida issue/ish; bu PR doirasidan tashqari).
3. **Sub-agent run-tekshiruviga ishonmaslik** — natijani asosiy agent o'zi
   verifikatsiya qilishi shart.

## Fayllar
- `flux-agent.builder.md` — agentga berilgan yangilangan spec
- `agent-output.fx` — agent yozgan to'liq backend (1 string-xatosi bilan)
- `validate.mjs` — validatsiya workflow manbai

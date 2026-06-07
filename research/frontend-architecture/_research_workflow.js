export const meta = {
  name: 'flux-prod-frontend-research',
  description: 'Production frontend arxitekturasi tadqiqoti: haiku zamonaviy yechimlarni qidiradi (Qwik/Marko/RSC/Astro/Solid), opus Flux uchun avtomatik SSR/client ajratish arxitekturasini sintez qiladi',
  phases: [
    { title: 'Research', detail: 'haiku × 5: har biri bitta zamonaviy yondashuvni chuqur qidiradi/fetch qiladi', model: 'haiku' },
    { title: 'Synthesize', detail: 'opus: hammasini Flux uchun statik-tahlil avtomatik-ajratish arxitekturasiga sintez qiladi', model: 'opus' },
  ],
}

// FAZA 1 — qidiruv (HAIKU, arzon). Har agent bitta yondashuvni chuqur o'rganadi.
// Diqqat: HAR BIRI "avtomatik SSR/client ajratish QANDAY ANIQLANADI" ga fokus.
const TOPICS = [
  {
    key: 'qwik',
    label: 'qidiruv: Qwik resumability',
    q: `Qwik framework resumability va progressive hydration. ANIQ shu savollarga javob top:
1. Qwik qanday qilib AVTOMAT ravishda kod qaysi qismi serverda, qaysi qismi browserda (lazy) ishlashini ajratadi? Statik tahlil (compiler/optimizer) qanday ishlaydi?
2. "$" suffiks (component$, onClick$) nima — compiler buni qanday lazy-loadable bo'lakka ajratadi?
3. Resumability nima (hydration'dan farqi) — serializatsiya qanday, server holatini browser qanday "davom ettiradi"?
4. Qwik Optimizer (Rust) kodni qanday bo'laklarga (chunks) ajratadi — symbol extraction?
5. Event handler'lar qanday lazy yuklanadi (faqat kerak bo'lganda)?`,
  },
  {
    key: 'marko',
    label: 'qidiruv: Marko auto partial hydration',
    q: `Marko (eBay) framework automatic partial hydration va streaming. ANIQ javob top:
1. Marko qanday qilib AVTOMAT ravishda qaysi komponent interaktiv (client JS kerak), qaysi biri sof statik (faqat SSR HTML) ekanini compiler bilan aniqlaydi?
2. Marko compiler kodni statik tahlil qilib "stateful" vs "stateless" qismni qanday ajratadi? Dasturchi belgilamaydi — qanday avtomat?
3. Marko 6 / Marko Run tags-api: reaktivlik (state, <const>, <let>) qanday compile bo'ladi?
4. Streaming SSR + partial hydration qanday birga ishlaydi?
5. "Only ship JS for interactive parts" — bu qanday amalga oshiriladi (bytecode/serialization)?`,
  },
  {
    key: 'rsc',
    label: 'qidiruv: React Server Components',
    q: `React Server Components (RSC). ANIQ javob top:
1. RSC server-component vs client-component ni qanday AJRATADI? "use client" direktivasi qanday ishlaydi — chegara (boundary) qanday aniqlanadi?
2. Server komponent kodi browserga UMUMAN yuborilmaydi — bu qanday amalga oshiriladi (bundler/compiler darajasida)?
3. Server -> client ma'lumot uzatish (serialization, RSC payload/wire format) qanday?
4. Server komponent ichida client komponent (va aksincha) qanday joylashadi — kompozitsiya modeli?
5. Data fetching server komponentda (async) — bu xavfsizlik (DB/secret client'ga chiqmaydi) qanday ta'minlanadi?`,
  },
  {
    key: 'astro-solid',
    label: 'qidiruv: Astro islands + Solid signals',
    q: `Astro islands arxitekturasi va SolidJS fine-grained signals. ANIQ javob top:
1. Astro "islands architecture" — sahifa default STATIK (0 JS), faqat island'lar (client:load/client:visible) interaktiv. Bu qanday aniqlanadi va compile bo'ladi?
2. Astro qanday qilib statik HTML + minimal island JS ni ajratadi? client:* direktivalar qanday ishlaydi?
3. SolidJS fine-grained signals: virtual-DOM YO'Q, signal o'zgarsa faqat bog'liq DOM yangilanadi. Compiler JSX'ni qanday reaktiv DOM-update kodga aylantiradi?
4. Solid SSR + hydration qanday (resumability emas, lekin fine-grained)?
5. Partial/selective hydration island darajasida qanday optimal qilinadi?`,
  },
  {
    key: 'compiler-analysis',
    label: 'qidiruv: statik tahlil reaktivlik',
    q: `Frontend compiler statik tahlil orqali reaktivlik va server/client ajratish. ANIQ javob top:
1. Svelte compiler reaktivlikni qanday statik tahlil qiladi ($: reactive, runes $state/$derived)? Compile vaqtida dependency graph qanday quriladi?
2. Compiler qanday qilib bir kod bo'lagi "interaktivlik talab qiladimi yoki yo'qmi" ni aniqlaydi (event handler, mutable state izi)? Dataflow/taint analysis?
3. "Islands" yoki "server/client boundary" ni AVTOMAT (dasturchi belgilamasdan) aniqlash bo'yicha tadqiqotlar/yondashuvlar bormi?
4. Reaktiv dependency graph (qaysi state qaysi DOM'ga ta'sir qiladi) compile vaqtida qanday quriladi?
5. Tree-shaking + dead-code elimination server-only kodni client bundle'dan qanday chiqaradi?`,
  },
]

phase('Research')

const findings = await parallel(
  TOPICS.map((t) => () =>
    agent(
      `Sen frontend arxitektura tadqiqotchisisan. Web'dan qidirib (WebSearch/WebFetch ishlatib), quyidagi mavzuni CHUQUR o'rgan va aniq, texnik, manbalar bilan javob ber.\n\n${t.q}\n\nJavob: har savolga konkret texnik javob (qanday ISHLAYDI, mexanizm darajasida), kod/sintaksis misollari bilan. O'zbek tilida yoz. Manbalarni (URL) ko'rsat. Faktlarga sodiq qol — bilmasang "aniq topilmadi" deb yoz, to'qima.`,
      { label: t.label, phase: 'Research', model: 'haiku' }
    ).then((text) => ({ key: t.key, text }))
  )
)

const valid = findings.filter(Boolean)
log(`Qidiruv tugadi: ${valid.length}/${TOPICS.length} mavzu`)

phase('Synthesize')

// FAZA 2 — sintez (OPUS, faqat shu yerda — o'ylash ishi).
const SYNTH = `Sen Flux dasturlash tilining bosh arxitektorisan. Flux — AI-native til (Rust tree-walking interpreter), backend TAYYOR (http/db/ai/ws/...). Frontend qatlami qurilmoqda: 1-3 bosqich TUGADI (view/theme/page/each/if/match/ui.serve — server-side render to'liq ishlaydi). Element = {__node} map, view = Value::Fn, hozir har request interpreter SSR qiladi.

ENDI ENG MUHIM QAROR: foydalanuvchi PRODUCTION, KATTA TRAFIKLI web saytlar yozish imkonini xohlaydi. Talab: Flux kod AVTOMAT ravishda (dasturchi belgilamasdan, STATIK TAHLIL orqali) qaysi qism SSR'da, qaysi qism faqat browser JS'da ishlashini O'ZI ajratsin. "Hammasini flux->js" yoki "html cache" — RAD etilgan (yuzaki). Kerak: aqlli avtomatik ajratish.

Quyida 5 ta zamonaviy yondashuv bo'yicha tadqiqot natijalari (Qwik resumability, Marko auto partial hydration, React Server Components, Astro islands+Solid signals, compiler statik tahlil). Ularni o'qib, Flux uchun ANIQ, AMALGA OSHIRILADIGAN arxitektura loyihalashtir.

===== TADQIQOT NATIJALARI =====
${valid.map((f) => `\n##### ${f.key} #####\n${f.text}`).join('\n')}
===== TADQIQOT OXIRI =====

Flux kontekstini yodda tut:
- Flux falsafasi: bir ish = bir yo'l, kam token, AI yaxshi yozadi, mavjud idioma qayta ishlatish.
- Flux'da \`<-\` = mutable/reaktiv state, \`=\` = immutable, \`on:\` = event, \`source\` = reaktiv data (db/http), \`act\` = view-handler, \`each\`/\`if\`/\`match\` = render.
- Element {__node} map, view Value::Fn (interp).
- Runtime Rust (lexer/parser/interp). Yangi: ui_mod.rs (element/serve), node_to_html (SSR).

Quyidagilarni ANIQ loyihalashtir (o'zbek tilida, konkret):

1. AVTOMATIK AJRATISH MEXANIZMI (eng muhim) — Flux compiler/analyzer qanday STATIK TAHLIL bilan aniqlaydi: qaysi view/qism sof statik (SSR, 0 JS), qaysi qism interaktiv (client JS kerak)? Aniq qoida: \`<-\`/\`on:\`/\`act\` izi bo'lgan qism client island bo'ladimi? \`source\` server'da qoladimi? Dependency/reaktivlik grafini qanday quramiz? Dataflow tahlil qanday? Misol Flux kodda qaysi qism qayerga ketishini ko'rsat.

2. CLIENT KOD QAYERDAN KELADI — Flux'ning interaktiv qismi browserda qanday ishlaydi? Variantlar: (a) Flux->JS transpile (interaktiv island'lar JS'ga compile), (b) universal Flux-runtime-in-JS (interpreter browserda), (c) gibrid. Qaysi biri Flux falsafasi + performance uchun to'g'ri? Resumability (Qwik) yoki hydration (Solid/Astro) — qaysi biri? DALIL bilan.

3. SERVER/CLIENT CHEGARASI — ma'lumot (source/db) qanday server'da qoladi (xavfsizlik, RSC uslubi)? Server qism client'ga UMUMAN yuborilmasligi qanday ta'minlanadi? Serializatsiya/wire-format qanday?

4. ARXITEKTURA + RUNTIME O'ZGARISHLARI — bu Flux runtime'ga (Rust) nimani qo'shadi? Yangi bosqich: AST tahlil (analyzer), kompilyatsiya bosqichi (view -> client chunk + server render), client runtime (JS). Mavjud interp/ui_mod bilan qanday birlashadi? Build pipeline qanday (\`flux build\`?, \`ui.serve\` production rejimi)?

5. PERFORMANCE — katta trafik uchun: statik qism CDN/kesh, interaktiv qism minimal JS, server render kompilyatsiyalanган (interpreter hot-path'dan chiqadi)? Aniq performance strategiyasi.

6. BOSQICHMA-BOSQICH REJA — buni mavjud 4-bosqich (reaktivlik) o'rniga/ustiga qanday quramiz? Realistik, PR-larga bo'lingan reja. Eng katta texnik xavflar.

7. SAVOLLAR/QARORLAR — foydalanuvchi hal qilishi kerak bo'lgan ochiq qarorlar (agar bo'lsa).

Bu eng muhim arxitektura hujjati — chuqur, konkret, amalga oshiriladigan bo'lsin.`

const architecture = await agent(SYNTH, {
  label: 'sintez: Flux prod frontend arxitektura',
  phase: 'Synthesize',
  model: 'opus',
})

return {
  findings: valid,
  architecture,
}

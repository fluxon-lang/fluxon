export const meta = {
  name: 'db-builder-validation',
  description: 'Re-run the booking+analytics PRD against the NEW (working) builder spec; a haiku agent writes .fx, we actually RUN it to check syntax/semantics, agent self-fixes once',
  phases: [
    { title: 'Generate', detail: 'haiku writes .fx from the builder spec' },
    { title: 'Run+Fix', detail: 'execute via flux binary; on error the agent fixes once' },
  ],
}

const prd = args.prd
const spec = args.spec
const fluxBin = args.fluxBin
const workDir = args.workDir

phase('Generate')

const GEN_SCHEMA = {
  type: 'object',
  required: ['fx_code', 'used_escape_hatch', 'escape_hatch_where', 'hardest_parts'],
  properties: {
    fx_code: { type: 'string', description: 'The full .fx backend, code only — no markdown fences' },
    used_escape_hatch: { type: 'boolean', description: 'Did you fall back to raw db.q/db.one SQL anywhere?' },
    escape_hatch_where: { type: 'array', items: { type: 'string' }, description: 'Which endpoints needed raw SQL' },
    hardest_parts: { type: 'array', items: { type: 'string' }, description: 'Which PRD parts were awkward in the builder syntax' },
  },
}

const gen = await agent(
  `You are writing a backend in the Flux language (.fx). Below is the COMPLETE Flux spec — the single source of truth for syntax. Follow it EXACTLY; do not invent syntax. Pay special attention to the db read builder (db.from |> db.eq |> ... |> db.all/first/agg).\n\n` +
  `===== FLUX SPEC =====\n${spec}\n===== END SPEC =====\n\n` +
  `Implement this product. Output the full .fx file (code only, no markdown).\n\n` +
  `===== PRD =====\n${prd}\n===== END PRD =====\n\n` +
  `Prefer the db builder for all reads/analytics; use raw db.q ONLY for true multi-table JOINs and date() grouping. Report honestly where you had to escape to raw SQL.`,
  { label: 'gen:builder', phase: 'Generate', model: 'haiku', schema: GEN_SCHEMA },
)

if (!gen) return { error: 'generation failed' }

phase('Run+Fix')

// Agent kodini faylga yozib, flux binary bilan PARSE-tekshiruvdan o'tkazadigan
// helper agent. flux'da serverni ishga tushirmasdan, faqat sintaksis/yuklashni
// tekshirish uchun kodning oxiridan http.serve ni olib tashlab "check" rejimida
// run qilamiz — bu parse + top-level eval (tbl, route reg) ni bajaradi.
const RUNCHECK_SCHEMA = {
  type: 'object',
  required: ['ok', 'stderr_tail'],
  properties: {
    ok: { type: 'boolean', description: 'Did the flux binary load the file without a parse/eval error?' },
    stderr_tail: { type: 'string', description: 'Last lines of stderr (the error, if any)' },
  },
}

// Bir urilishni run qiladigan agent (Bash bilan). http.serve ni "check" uchun
// olib tashlaydi — server bloklamasin, lekin qolgan hammasi (tbl, http.on
// route'lari, db builder chaqiruvlari top-level'da bo'lsa) baholanadi.
async function runCheck(code, tag) {
  return agent(
    `Write the following Flux code to ${workDir}/candidate.fx, then create a check copy ${workDir}/check.fx that is IDENTICAL except every line starting with \`http.serve\` is removed (so it won't block). Run:\n` +
    `  DATABASE_URL="sqlite::memory:" ${fluxBin} run ${workDir}/check.fx\n` +
    `Use a 20s timeout. Report ok=true ONLY if it exits 0 with no Flux error on stderr. Put the last ~15 lines of stderr in stderr_tail.\n\n` +
    `CODE:\n<<<FLUX\n${code}\nFLUX`,
    { label: `run:${tag}`, phase: 'Run+Fix', model: 'haiku', schema: RUNCHECK_SCHEMA },
  )
}

const first = await runCheck(gen.fx_code, 'attempt1')

let final = { fx_code: gen.fx_code, run: first, fixed: false, fix_run: null }

// Xato bo'lsa: agentga stderr ni ko'rsatib, BIR marta tuzattiramiz (bu "agent
// xatoni o'zi tushunib tuzata oladimi" ni o'lchaydi — issue'ning asl savoli).
if (first && !first.ok) {
  const FIX_SCHEMA = {
    type: 'object',
    required: ['fx_code', 'what_was_wrong'],
    properties: {
      fx_code: { type: 'string' },
      what_was_wrong: { type: 'string', description: 'One sentence: what the error was and the fix' },
    },
  }
  const fix = await agent(
    `Your Flux code failed to run. Here is the spec (db builder section matters most), your code, and the error. Fix it. Output the full corrected .fx.\n\n` +
    `===== SPEC =====\n${spec}\n===== END SPEC =====\n\n` +
    `===== YOUR CODE =====\n${gen.fx_code}\n===== END =====\n\n` +
    `===== ERROR (stderr) =====\n${first.stderr_tail}\n===== END =====`,
    { label: 'fix', phase: 'Run+Fix', model: 'haiku', schema: FIX_SCHEMA },
  )
  if (fix) {
    const second = await runCheck(fix.fx_code, 'attempt2')
    final = { fx_code: fix.fx_code, run: first, fixed: true, fix_note: fix.what_was_wrong, fix_run: second }
  }
}

return { generation: gen, validation: final }

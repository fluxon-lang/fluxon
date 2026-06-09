export const meta = {
  name: 'db-orm-design-bakeoff',
  description: 'Test 5 ORM design specs against a haiku agent on a hard booking+analytics backend; compare which the agent writes best in one read',
  phases: [
    { title: 'Generate', detail: 'haiku agent writes the .fx backend from each variant spec' },
    { title: 'Judge', detail: 'one analyst scores all five outputs side by side' },
  ],
}

// --- kirish materiallari (workflow args orqali keladi) ---
// args = { prd: "<PRD matni>", specs: { "v1-nested-map": "<to'liq spec>", ... } }
const prd = args.prd
const specs = args.specs
const variantNames = Object.keys(specs)

// Har variant uchun haiku agent: spec + PRD beriladi, .fx kod + o'z-hisobot qaytaradi.
const GEN_SCHEMA = {
  type: 'object',
  required: ['fx_code', 'self_report'],
  properties: {
    fx_code: { type: 'string', description: 'The full .fx backend, code only' },
    self_report: {
      type: 'object',
      required: ['hardest_parts', 'used_escape_hatch', 'confidence'],
      properties: {
        hardest_parts: {
          type: 'array', items: { type: 'string' },
          description: 'Which PRD requirements were hardest to express in this db syntax',
        },
        used_escape_hatch: {
          type: 'boolean',
          description: 'Did you fall back to raw db.q SQL for any read/analytics query?',
        },
        escape_hatch_where: {
          type: 'array', items: { type: 'string' },
          description: 'If yes, which endpoints needed raw SQL',
        },
        confidence: {
          type: 'number',
          description: '0..1 — how sure are you the db calls match the spec exactly',
        },
      },
    },
  },
}

phase('Generate')
const generations = await parallel(variantNames.map((name) => () =>
  agent(
    `You are writing a backend in the Flux language (.fx). Below is the COMPLETE Flux language spec — it is the single source of truth for syntax and batteries. Follow it exactly; do not invent syntax.\n\n` +
    `===== FLUX SPEC (variant: ${name}) =====\n${specs[name]}\n===== END SPEC =====\n\n` +
    `Now implement this product. Output the full .fx file.\n\n` +
    `===== PRD =====\n${prd}\n===== END PRD =====\n\n` +
    `Write the backend. In your self_report, be honest about which parts of the PRD were awkward to express with THIS spec's db syntax, and whether you had to drop to raw db.q SQL.`,
    { label: `gen:${name}`, phase: 'Generate', model: 'haiku', schema: GEN_SCHEMA },
  ).then((r) => ({ name, ...r })),
))

const ok = generations.filter(Boolean)

// Hamma natijani bitta tahlilchiga beramiz — yonma-yon solishtiradi.
phase('Judge')
const JUDGE_SCHEMA = {
  type: 'object',
  required: ['per_variant', 'winner', 'reasoning', 'recommendation'],
  properties: {
    per_variant: {
      type: 'array',
      items: {
        type: 'object',
        required: ['name', 'correctness', 'token_economy', 'readability', 'escape_hatch_use', 'notable'],
        properties: {
          name: { type: 'string' },
          correctness: { type: 'number', description: '0..10 — do the db calls actually satisfy the PRD (IN, range, group, order, paging, tx)' },
          token_economy: { type: 'number', description: '0..10 — fewer/cleaner tokens for the reads & analytics' },
          readability: { type: 'number', description: '0..10 — would another agent understand it in one read' },
          escape_hatch_use: { type: 'string', description: 'how often it fell back to raw db.q, and where' },
          notable: { type: 'array', items: { type: 'string' }, description: 'strengths / mistakes / misunderstandings of the syntax' },
        },
      },
    },
    winner: { type: 'string' },
    reasoning: { type: 'string' },
    recommendation: { type: 'string', description: 'Concrete proposal for what to actually implement in Flux db.* — may be a hybrid taking the best parts of several variants' },
  },
}

const bundle = ok.map((g) =>
  `### VARIANT ${g.name}\n` +
  `self_report: ${JSON.stringify(g.self_report)}\n\n` +
  `\`\`\`flux\n${g.fx_code}\n\`\`\``
).join('\n\n---\n\n')

const judge = await agent(
  `You are evaluating ${ok.length} candidate DB-query designs for the Flux language. Each was given the SAME PRD (a multi-tenant booking + analytics backend) and a Flux spec differing ONLY in how reads/aggregation are written. A haiku agent wrote each backend in ONE pass.\n\n` +
  `The goal: pick the db-query design that an AI agent writes most correctly and with fewest tokens in one read. The hardest signals are: did it get IN-filters right, time-range, GROUP-BY analytics, ordering+paging, and the race-safe booking transaction — WITHOUT dropping to raw SQL.\n\n` +
  `Here is the PRD:\n${prd}\n\n` +
  `Here are the ${ok.length} outputs:\n\n${bundle}\n\n` +
  `Score each variant, pick a winner, and give a concrete recommendation for what to implement in Flux's db.* (a hybrid is allowed). Be specific about which syntax choices helped or hurt the agent.`,
  { label: 'judge', phase: 'Judge', schema: JUDGE_SCHEMA },
)

return { generations: ok, judge }

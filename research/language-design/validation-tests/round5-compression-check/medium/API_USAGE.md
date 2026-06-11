# Polls API with AI Summary — Fluxon Implementation

Complete polls/survey API with AI-powered result summarization, written in Fluxon.

## Database Schema

Three tables:

- **polls** (id, owner, question, status:sym, created)
- **options** (id, poll_id ref, text, votes, created)
- **responses** (id, poll_id ref, option_id ref, voter, created)

All IDs are auto-generated serials. Status is a symbol: `:open` or `:closed`.

## API Endpoints

### 1. Create Poll
```
POST /polls
Content-Type: application/json

{
  "owner": "user@example.com",
  "question": "What is your favorite language?",
  "options": ["Fluxon", "Python", "Go", "Rust"]
}
```

Response (201):
```json
{
  "id": 1,
  "owner": "user@example.com",
  "question": "What is your favorite language?",
  "status": "open",
  "created": "2026-06-05T10:00:00Z"
}
```

### 2. Get Poll with Options
```
GET /polls/:id
```

Response (200):
```json
{
  "id": 1,
  "owner": "user@example.com",
  "question": "What is your favorite language?",
  "status": "open",
  "created": "2026-06-05T10:00:00Z",
  "options": [
    {"id": 1, "poll_id": 1, "text": "Fluxon", "votes": 5, "created": "..."},
    {"id": 2, "poll_id": 1, "text": "Python", "votes": 3, "created": "..."},
    {"id": 3, "poll_id": 1, "text": "Go", "votes": 2, "created": "..."},
    {"id": 4, "poll_id": 1, "text": "Rust", "votes": 4, "created": "..."}
  ],
  "total_votes": 14
}
```

### 3. Cast a Vote
```
POST /polls/:id/vote
Content-Type: application/json

{
  "option_id": 1,
  "voter": "alice@example.com"
}
```

Response (200):
```json
{
  "success": true,
  "option_id": 1
}
```

Validates:
- Poll exists
- Poll status is `:open`
- Option exists and belongs to the poll

### 4. Close Poll
```
POST /polls/:id/close
```

Response (200):
```json
{
  "success": true,
  "status": "closed"
}
```

### 5. Get AI Summary
```
POST /polls/:id/summarize
```

Uses `ai.ask` to generate a natural language summary of results:

Response (200):
```json
{
  "poll_id": 1,
  "question": "What is your favorite language?",
  "summary": "Fluxon was the clear winner with 5 votes (36%), followed by Rust with 4 votes (29%), Python with 3 votes (21%), and Go with 2 votes (14%)."
}
```

### 6. List Polls
```
GET /polls?status=open
GET /polls?status=closed
GET /polls
```

Response (200):
```json
{
  "polls": [
    {
      "id": 1,
      "owner": "user@example.com",
      "question": "...",
      "status": "open",
      "created": "...",
      "total_votes": 14
    }
  ]
}
```

## Background Jobs

### Hourly Cron Task
Every hour at :30 minutes, logs:
- Count of active (open) polls
- Total votes across all open polls
- Timestamp

Example log output:
```
HOURLY STATS: 3 active polls, 42 total votes at 2026-06-05T11:30:00Z
```

## Implementation Features

1. **Validation** — All input validated; returns 400 for missing/invalid fields
2. **Transactions** — Vote casting uses `db.tx` to atomically insert response and increment vote count
3. **Error Handling** — Proper HTTP status codes (404 for not found, 422 for poll closed, 400 for validation)
4. **Symbols** — Poll status stored as DB symbol, auto-converted between text (DB) and symbol (Fluxon)
5. **AI Integration** — `ai.ask` generates natural language summaries of poll results
6. **Cron Jobs** — Automatic hourly statistics logging
7. **Module Organization** — Split into schema, models, API, AI, and cron tasks

## Running

```bash
export DATABASE_URL=postgres://user:pass@localhost/polls_db
export AI_KEY=sk-...
bun run main.fx
```

Server starts on port 8080.

Ajoyib! Endi barcha ma'lumotlarni to'plantirib, aniq teknik javoblarni yaratamiz:

---

# React Server Components (RSC) — CHUQUR TEKNIK TADQIQOT

## 1️⃣ RSC va Client Component Separation: 'use client' Direktivi va Boundary

### Qanday Ajratiladi?

`'use client'` direktivasi **module dependency tree** (import munosabati) asosida, **render tree** emas asosida chegarani yaratadi:

```tsx
// App.js — Server Component (default)
import ServerHeader from './ServerHeader';      // Server
import ClientButton from './ClientButton';      // 'use client' → Client

export default function App() {
  return (
    <>
      <ServerHeader />
      <ClientButton />  {/* Render tree: button App-ning child */}
    </>
  );
}
```

**Muhim fark**: Render tree-dagi parent-child munosabati irrelevant. Agar `ClientButton.js` faylining **boshi** `'use client'` direktivasi bo'lsa — u client komponentlari hisoblanadi, render pozitsiyasiga qaramay.

### Boundary Aniqlanishi

```
MODULE DEPENDENCY TREE (Import):
  App.js (Server)
  ├── ClientButton.js ('use client') ← BOUNDARY!
  │   ├── useState.js (Client)
  │   └── utils.js (Client)
  └── ServerHeader.js (Server)
      └── db.js (Server)
```

**Qoida**: `'use client'` qo'yilgan modul va uning barcha transitiv importlari (`import` langanlar) client bundleiga kiritiladi. Bundler ularni **build time**da static analysis orqali aniqlaydi.

### "use client" Direktivi Qanday Ishlaydi

- **Build vaqtida**: Bundler `'use client'` direktivasini tushunadi va o'sha modul + barcha bog'liq dependencies ni client bundle ga kiritadi
- **Runtime vaqtida**: Server tomonidan client komponentlardan faqat "Client Reference" (metadata) yuboriladi, asl kod yo'q

---

## 2️⃣ Server Kodi Browserga Yuborilmaydi — Bundler Mexanizmi

### Qanday Ta'minlanadi?

Server Components faqat **rendered output** (HTML) browserga yuboriladi. Asl komponent kodi **umuman bundle ga kiritilmaydi**.

**Misolda**:

```tsx
// Server Component — Bundledan OLIB TASHLANDI
async function BlogPost({ id }) {
  import marked from 'marked';              // 35.9K ✂️
  import sanitizeHtml from 'sanitize-html'; // 206K ✂️
  
  const content = await db.posts.get(id);
  const html = sanitizeHtml(marked(content));
  
  return <article>{html}</article>;
}

// Browser ko'radi:
// <article><h1>Post Title</h1><p>Content...</p></article>
```

### Bundler Qanday O'chiradi?

1. **Static Analysis**: Bundler `'use client'` direktivas ni qayta ijro qilmasa, o'sha modul server-only deb belgilaydi
2. **Tree-shaking**: Bundler server-only kod va uning dependency larini client bundle dan o'chiradi
3. **Separation**: Parcel, Next.js va boshqa bundlerlar "unified module graph" asosida server va client kodini ajratadi:

```
UNIFIED MODULE GRAPH (Parcel/Next.js):

┌─────────────────────────────┐
│  Server Bundle              │
├─────────────────────────────┤
│ - BlogPost component code   │
│ - marked (35.9K)            │
│ - sanitizeHtml (206K)       │
│ - DB queries                │
│ - Secrets/API keys          │
└─────────────────────────────┘
        ↓ (rendered)
    HTML output
        ↓
    Send to client
        ↓
┌─────────────────────────────┐
│  Client Bundle (0 bytes)    │
├─────────────────────────────┤
│ - React runtime             │
│ - Client Components only    │
│ - Interactivity             │
└─────────────────────────────┘
```

**Natija**: Client bundle 75KB+ kichikroq, tezroq loading.

---

## 3️⃣ Server ↔ Client Ma'lumot Uzatish: Serialization va RSC Payload

### RSC Payload Nima?

RSC Payload — bu server komponentdan client komponentga o'tadigan **serialized props** va rendered component tree. Iç React internals da "Flight" protokoli deyiladi.

### Flight Protocol Format

```
<chunk_id>:<tag><json_payload>

Misol:
0:["$","div",null,{"children":"Hello"}]
1:["$","button",null,{"onClick":"$2"}]
a:{"testSet":"$W9","promise":"$@a"}
```

**Chunk ID** — heksadetsimal ID, har bir ma'lumot qismi alohida chunk
**Tag** — ma'lumot turi:
- `$` — React element
- `$W` — Set
- `$@` — Promise  
- `T` — Long string (1KB+), binary format

### Serialization Misolda

Server Components:

```tsx
async function Page() {
  const data = { name: 'Ali', items: new Set(['a', 'b']) };
  const promise = fetchData();
  
  return <ClientComp data={data} promise={promise} />;
}
```

RSC Payload:

```json
{
  "testData": "$a",
  "testPromise": "$@b"
}

// Chunk "a":
{"name": "Ali", "items": "$W9"}

// Chunk "W9": 
["a", "b"]

// Chunk "@b":
// (pending promise)
```

**Client deshifratsiya**:
- `$W9` → `Set(['a', 'b'])` qayta tiklandi
- `$@b` → pending promise awaited
- Barcha ma'lumot o'z turida restore bo'ladi

### Streaming Optimalasyon

Production RSC setup:
- Server JSX chunks stream qiladi real-time (hamma sekaligina emas)
- Client hydration boshlanadi darhol
- Network waterfallning oldini olinadi

---

## 4️⃣ Komponentlar Nesting: Server & Client Pozitsiyalari

### Nesting Qoidalari

#### ✅ Server → Client (Lekin props serializable bo'lishi kerak)

```tsx
// Server Component
async function Page() {
  const userData = await db.users.get(id);
  
  // ✅ ISHLAYDI: User data props orqali client ga o'tadi
  return <ClientProfile user={userData} />;
}

// Client Component  
'use client';
function ClientProfile({ user }) {
  return <div>{user.name}</div>;
}
```

**Shartlar**:
- Props serializable bo'lishi kerak (JSON, Set, Map, Promise, JSX — funksiya emas!)
- Secrets/tokens props orqali yo'q

#### ❌ Client → Server Direct Import (Haram!)

```tsx
'use client';

// ❌ XATO: Client component server componentni import qila olmaydi
import ServerData from './ServerData'; // ERROR!

export function ClientComp() {
  return <ServerData />; // Bundler olib tashlaydi
}
```

**Sabab**: Client Componentdan import qilingan hamma code client bundleiga kiritiladi. Server kodi browser da ishlamaydi.

#### ✅ Client ichiga Server Komponent (Prop orqali!)

```tsx
'use client';

// ✅ Server komponentni PROP sifatida kabul qil
function Modal({ children }) {
  const [open, setOpen] = useState(false);
  return (
    <>
      {open && children} {/* children = Server Component */}
      <button onClick={() => setOpen(!open)}>Toggle</button>
    </>
  );
}
```

```tsx
// Server Component  
import Modal from './Modal'; // Client
import ServerData from './ServerData'; // Server

export default function Page() {
  // ✅ ServerData Modal-ning children prop sifatida
  return (
    <Modal>
      <ServerData /> {/* Server render qiladi, props JSON sifatida */}
    </Modal>
  );
}
```

**Mexanizm**: Server komponenti render qiladi, HTML output → JSON serialization → Modal-ga props → browser-da render

### Ownership/Composition

```
Page (Server)
├── Layout (Client) — state, click handlers
│   └── children prop (Server komponent kiritiladi)
│       └── BlogPost (Server) — async fetch, DB
│           └── Comments (Client) — like button
```

---

## 5️⃣ Server Komponentda Async Data Fetching: Security & Secrets Himoya

### Secrets Client-ga Chiqmaydi — Qanday?

**Asosiy prinsip**: Server komponent body-dagi kod **hech qachon client bundleiga kiritilmaydi**. Secrets, DB credentials, API keys ushbu code-da qoladi.

```tsx
// Server Component — BUNDLEDAN OLIB TASHLANDI
async function UserProfile({ userId }) {
  // ❌ API_SECRET server-only, client bundleiga kiritilmadi
  const apiSecret = process.env.API_SECRET;
  
  // ❌ DB query kodi browser-da ishlamaydi
  const user = await db.query(
    `SELECT * FROM users WHERE id = $1`,
    [userId]
  );
  
  // ✅ Faqat user.name va user.email serialized props orqali yuboriladi
  return (
    <div>
      <h1>{user.name}</h1>
      <p>{user.email}</p>
      {/* user.apiKey ASLA props ga kiritilmadi */}
    </div>
  );
}
```

**Client ko'radi**:
```html
<!-- Secrets yo'q, faqat rendered HTML -->
<div>
  <h1>Ali</h1>
  <p>ali@example.com</p>
</div>
```

### Security Patterns

#### 1️⃣ **Data Access Layer (Recommended)**

```tsx
// lib/auth.ts (server-only)
import 'server-only'; // ← Build error agar client-dan import qilsa

export async function getCurrentUser() {
  const token = cookies().get('AUTH_TOKEN')?.value;
  const decoded = await decryptToken(token); // Secrets qo'llanildi
  
  // Return minimal DTO (Data Transfer Object)
  return new User(decoded.id); // Faqat ID, secrets yo'q
}
```

```tsx
// lib/posts.ts (server-only)
import 'server-only';

export async function getPostDTO(slug: string) {
  // Direct DB access, secret queries
  const [rows] = await sql`SELECT * FROM posts WHERE slug = ${slug}`;
  const post = rows[0];
  
  // Faqat public fields return qil
  return {
    title: post.title,
    content: post.content,
    authorName: post.author_name,
    // ❌ post.internal_id, post.cost yo'q
  };
}
```

```tsx
// Page.tsx
import { getPostDTO } from '@/lib/posts';

export default async function Page({ params }) {
  const post = await getPostDTO(params.slug);
  // post barcha maydonlari serializable va xavfsiz
  return <PostViewer post={post} />;
}
```

#### 2️⃣ **React Taint API (Experimental)**

```tsx
// lib/data.ts
import { experimental_taintObjectReference } from 'react';

export async function getUserData(id) {
  const user = await db.users.get(id);
  
  // ❌ Build vaqtida: agar ushbu object client-ga o'tkazsa ERROR!
  experimental_taintObjectReference(
    'User data must not reach client',
    user
  );
  
  return user;
}
```

```tsx
// Page.tsx
const user = await getUserData(id);
return <ClientComp user={user} />; // ❌ COMPILE ERROR!

// Lekin field extrapolation hali ham risking:
const { name, phone } = user;
return <ClientComp name={name} phone={phone} />; // ⚠️ Taint yo'q
```

#### 3️⃣ **"server-only" Package**

```tsx
// lib/secrets.ts
import 'server-only'; // ← NPM package

const dbConnection = createConnection(process.env.DB_SECRET);

export async function fetchSecureData() {
  return await dbConnection.query(...);
}
```

Agar client-dan import qilsa:
```tsx
'use client';
import { fetchSecureData } from '@/lib/secrets'; // ❌ BUILD ERROR!
```

Bundler build-da hato toshlaydi — dastlab client-da ishlamaydi.

### Async/Await — Serialization Ichida

```tsx
async function Page() {
  // ✅ Async/await server-da ishlaydi
  const post = await db.posts.get(id);
  const comments = await db.comments.get(post.id);
  
  // Promise combine:
  const data = await Promise.all([
    db.author.get(post.authorId),
    db.tags.get(post.id),
  ]);
  
  // ✅ Barcha natijalar serialized props sifatida client-ga yuboriladi
  return <PostView post={post} comments={comments} author={data[0]} />;
}
```

---

## 📋 XULOSA JADVALI

| Savollar | Javoblar |
|---------|---------|
| **'use client' boundary?** | Module dependency tree asosida (import), render tree emas. Build-time static analysis. |
| **Server kod browserga?** | **Umuman yuborilmaydi**. Bundler tree-shaking orqali olib tashlaydi. Faqat HTML output. |
| **Serialization?** | Flight protocol (line-based format): `<id>:<tag><json>`. Sets, Maps, Promises, JSX — bu tiklandi. Funksiya emas. |
| **Nesting models?** | Server→Client (props), Client cannot import Server (error), Client→Server via children prop OK. |
| **Secrets ta'min?** | Server code server-only da qoladi. Faqat DTO (minimal fields) props ga o'tadi. Taint API, 'server-only' package. |

---

## 🔗 Manbalar

- [React 'use client' Reference](https://react.dev/reference/rsc/use-client)
- [React Server Components Reference](https://react.dev/reference/rsc/server-components)
- [Next.js: Server and Client Components](https://nextjs.org/docs/app/getting-started/server-and-client-components)
- [Next.js: Data Security Guide](https://nextjs.org/docs/app/guides/data-security)
- [Next.js: Security & Server Components](https://nextjs.org/blog/security-nextjs-server-components-actions)
- [Josh W. Comeau: Making Sense of RSC](https://www.joshwcomeau.com/react/server-components/)
- [React RFC #188: Server Components](https://github.com/reactjs/rfcs/blob/main/text/0188-server-components.md)
- [Parcel: RSC Bundling Strategy](https://devongovett.me/blog/parcel-rsc.html)
- [RSC Payload & Serialization (hrtyy.dev)](https://hrtyy.dev/web/rsc_payload/)
- [DebugBear: Introduction to RSC](https://www.debugbear.com/blog/react-server-components)
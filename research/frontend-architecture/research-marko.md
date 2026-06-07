Mukammal ma'lumotlar to'plandı. Endi javob yozaman.

---

# Marko Framework: Automatic Partial Hydration va Streaming SSR Texnik Tahlil

## 1. AVTOMAT RAVISHDA STATEFUL VS STATELESS TASNIF (Compiler Mexanizmi)

Marko compiler **statik tahlil (static analysis)** orqali qaysi kompanent interaktiv (client JS kerak), qaysi biri sof statik (faqat SSR HTML) ekanini avtomatik aniqlaydi.

**Qanday ishlaydi:**
- Compiler template kodini cross-file tahlil qiladi va reactive dependency graph tuzadi
- `<let>` tagi ishlatilgan komponentlar **stateful** deb belgilanadi (client-side state)
- `<let>` tagi yo'q komponentlar **stateless** deb aniqlanadi (sof HTML render)
- Compiler parent-child hierarchy'ni tahlil qilib qaysi komponentga client JS kerak, qaysiga yo'qligini aniqlaydi
- **Key point:** Dasturchi belgilamaydi - compiler avtomatik qaror beradi

**Natija:** Agar tundagi qism sof statik bo'lsa, JavaScript yuborilmaydi.

---

## 2. STATEFUL VS STATELESS AJRATISH MEXANIZMI (Compiler Analizi)

Marko compiler **4 ta alohida export**ga bo'ladi har komponent:

```
1. HTML Template (markup)
2. Walks - DOM traversal instructions (encoded format: "D%c%c%l" kabi)
3. Update functions (har input/prop uchun alohida)
4. Scope object (barcha reactive points of interest)
```

**Stateful komponent (client JS yuboriladi):**
```marko
<let name = "John">
<let count = 0>

<button @click() => { count++ }>
  Click me: ${count}
</button>
```
Compiler bu komponentni client-side include qiladi - `<let>` variabllar reactive.

**Stateless komponent (faqat SSR HTML):**
```marko
<div>
  <h1>${input.title}</h1>
  <p>${input.description}</p>
</div>
```
Client JS yuborilmaydi - props uchun input-based update function kerak emas.

**Compiler key mexanizmi:**
- Component boundary'sini **flatten** qiladi (closure'lari olib tashlaydi)
- "Scope object" - barcha reactive state va reference'larni closure o'rnida saqlaydi
- Tree-shakable exports: parent komponent qaysi child export'larni kerakligini compiler aniqlaydi

---

## 3. MARKO 6 TAGS API - REAKTIVLIK COMPILATION

Marko 6 yeni **Tags API** bilan reactivity compile-time'da tahlil qilinadi:

```marko
<let count = 0>           <!-- Mutable state -->
<const doubled = count * 2>  <!-- Derived reactive value -->
<effect>                      <!-- Side effects -->
  console.log(doubled)
</effect>

<button @click() => { count++ }>
  Count: ${count}, Doubled: ${doubled}
</button>
```

**Compilation sifatida:**
- Compiler `count` va `doubled` o'rtasidagi dependency'sini static tahlil qiladi
- Update function's minimal bo'ladi - faqat `count` o'zgarganida `doubled` recalculate qiladi
- "Scope" object'iga barcha state qiymatlar saqlanadi (closure o'rnida)
- Fine-grained update: tola komponentni re-render qilanmasdan, faqat dependency'si o'zgargan expression'lar update qiladi

**Output Example:**
```javascript
// Compiled output misoli:
export const template = /* HTML string */;
export const walks = "D%c%c%l"; // DOM traversal
export const updateCount = (scope, value) => {
  scope.count = value;
  scope.doubled = value * 2; // Automatic dependency update
};
```

---

## 4. STREAMING SSR + PARTIAL HYDRATION ISHLASH

Marko streaming va partial hydration quyidacha birga ishlaydi:

**Server Taraf:**
1. Synchronous content darhol stream qiladi
2. Async boundary'da placeholder o'rnatadi
3. Data kelganida, server-side render qiladi va `<script>` tag orqali yuboradi
4. **Key:** Sof statik HTML bo'lgan komponentlar clientga **JavaScript kodsiz** yuboriladi

**Client Taraf:**
1. HTML progressive render qiladi (browser eagerly render qiladi - closing tag kutmaydi)
2. Nur interaktiv komponentlarning JavaScript kodi download qiladi
3. Serialized state data'si script tag'da keladi
4. Progressive hydration: chunks arriving order'ida mount qiladi

**Mexanizm:**
```
Server: <div>Static Header</div> --> Client: Render (0 JS kerak)
Server: <div id="placeholder"></div> + <script>/* async data */</script>
Server: <button @click...><!-- Interactive Component --></button> + <script>/* hydration code */</script>
Client: Button script download --> Only button component's JS load qiladi
```

---

## 5. "FAQAT INTERAKTIV QISMLARGA JS YUBORISH" - ARXITEKTURA

Bu 4-qadamda amalga oshiriladi:

**Bytecode/Serialization Strategy:**

1. **Static Analysis Phase:**
   - Compiler template'larni analyze qiladi: qaysi expression dynamic, qaysi static
   - Dependency graph tuzadi (which state affects which DOM nodes)

2. **Serialization (Server -> Browser):**
   - Server **faqat interactive component'larning state'ini** JSON/binary format'ida serialize qiladi
   - Static component'larning data'si serialize qilinmaydi (waste yo'q)
   - Serialization format: `{componentId: "X", state: {...}, updates: [...]}`

3. **Hydration Bundling:**
   - Bundler (Vite/Rollup) tree-shaking qiladi
   - Faqat interactive komponentlarning JS bundle'da qolib qoladi
   - Static komponentlarning runtime code'i completely bundle'dan olib tashlanadi

4. **Runtime Deserialization:**
   - Browser serialized state'ni deserialize qiladi
   - Scope object'i initialize qiladi
   - Update function'lar attach qiladi DOM'ga

**Misal:**
```html
<!-- Server-rendered Static Component -->
<header>
  <h1>eBay - Header</h1>
</header>
<!-- 0 bytes JS for this header -->

<!-- Server-rendered Interactive Component -->
<div id="cart-island">
  <button @click...>Add to Cart</button>
</div>
<script>
  // FAQAT bu script'dan JS kerak
  __HYDRATION_DATA = {
    cartId: "island-1",
    state: { count: 0 },
    template: "...",
    updateFn: function(scope, action) { /* ... */ }
  };
</script>
```

---

## 6. COMPILER TECHNICAL IMPLEMENTATION

Marko compiler internal'ida:

- **Component Flattening:** Child komponentlarni parent template'iga inline qiladi (runtime overhead yo'q)
- **Scope Objects:** All closure's removed → scope object'i (serializable, fast)
- **Tree-Shakable Exports:** Har komponent multiple export'larga bo'linadi:
  - `template` - HTML
  - `walks` - encoded DOM navigation
  - `updateX` - input-specific update function (faqat keraksa)
  
- **Cross-Template Analysis:** Compiler file boundary'lari cross qiladi - whole app'ning dependency graph tuzadi
- **Field-level Granularity:** Serialization field-by-field qiladi (object'ning barcha property'si emas, faqat used property's)

---

## MANBALAR

- [Marko: Compiling Fine-Grained Reactivity - DEV Community](https://dev.to/ryansolid/marko-compiling-fine-grained-reactivity-4lk4)
- [eBay's UI Framework Marko Adds Optimized Reactivity Model - InfoQ](https://www.infoq.com/articles/ebay-marko-performance-reactivity-model/)
- [Introducing the Marko Tags API Preview - DEV Community](https://dev.to/ryansolid/introducing-the-marko-tags-api-preview-37o4)
- [What has the Marko Team Been Doing all These Years - DEV Community](https://dev.to/ryansolid/what-has-the-marko-team-been-doing-all-these-years-1cf6)
- [Server Rendering in JavaScript: Optimizing for Size - DEV Community](https://dev.to/ryansolid/server-rendering-in-javascript-optimizing-size-3518)
- [Why Efficient Hydration in JavaScript Frameworks is so Challenging - DEV Community](https://dev.to/this-is-learning/why-efficient-hydration-in-javascript-frameworks-is-so-challenging-1ca3)
- [Marko GitHub - marko-js/run](https://github.com/marko-js/run)
- [Core Tags | Marko Documentation](https://markojs.com/docs/reference/core-tag)
- [Islands Architecture - patterns.dev](https://www.patterns.dev/vanilla/islands-architecture/)
- [Streaming Server-Side Rendering - patterns.dev](https://www.patterns.dev/react/streaming-ssr/)
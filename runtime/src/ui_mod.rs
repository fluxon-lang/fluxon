// Flux frontend (UI qatlami) — 1-BOSQICH (MVP): statik element daraxti -> HTML.
//
// Falsafa (docs/flux-frontend.md): UI backend bilan BIR `.fx` faylda. `view` =
// `fn`ning UI varianti, element daraxti qaytaradi. Element YANGI Value variant
// TALAB QILMAYDI — `http_mod`ning `{__resp:true ...}` idiomasi takrorlanadi:
// element = maxsus kalitli map `{__node:true tag:"div" text:.. props:{..} children:[..]}`.
// Bu Send+Sync invariantini avtomatik saqlaydi (value.rs tegilmaydi).
//
// MVP doirasi: element konstruktorlari (`div`/`p`/`h1`/...), `node_to_html` (SSR),
// `ui.html node -> str`. Reaktivlik/server/source — keyingi bosqichlar.

use std::collections::BTreeMap;
use std::sync::Arc;

use crate::interp::{Flow, Interp};
use crate::value::Value;

// MVP'da qo'llab-quvvatlanadigan core HTML teglari. Semantik proplar bularning
// ustida ishlaydi; ro'yxat keyingi bosqichlarda kengayadi.
//
// Teglar GLOBAL O'ZGARUVCHI EMAS: `a`, `p`, `form`, `input` kabi nomlar keng
// tarqalgan o'zgaruvchi nomlari (`a = 5`). Shuning uchun ular faqat CHAQIRUV
// pozitsiyasida (callee) va nom band bo'lmaganda element sifatida hal qilinadi
// (interp::eval_call). Bu oddiy bind bilan to'qnashuvni butunlay yo'qotadi.
const CORE_TAGS: &[&str] = &[
    "div", "p", "h1", "h2", "h3", "span", "btn", "img", "input", "a", "ul", "li", "form", "badge",
];

// Nom element teg ekanmi (interp eval_call fallback uchun).
pub fn is_element_tag(name: &str) -> bool {
    CORE_TAGS.contains(&name)
}

// Element teg chaqirig'ini ({__node} map) quradi — interp eval_call'dan.
pub fn build_element(tag: &str, args: Vec<Value>) -> Result<Value, Flow> {
    build_node(tag, args)
}

// Element argumentlarini o'qib `{__node:true tag:.. text:.. props:.. children:..}`
// map quradi. Argumentlar tartibi erkin (spec: `tag content {props}`):
//   - str/int/sym/bool/flt qiymat -> text (matn bola)
//   - map -> props
//   - list -> children (boshqa elementlar; parser oxirgi argument sifatida beradi)
fn build_node(tag: &str, args: Vec<Value>) -> Result<Value, Flow> {
    let mut text: Option<String> = None;
    let mut props: BTreeMap<String, Value> = BTreeMap::new();
    let mut children: Vec<Value> = Vec::new();

    for a in args {
        match a {
            Value::Map(m) => {
                // {__node} bo'lsa — bu bola element (qavs ichida yozilgan bo'lsa).
                if is_node(&Value::Map(m.clone())) {
                    children.push(Value::Map(m));
                } else {
                    props = m;
                }
            }
            Value::List(items) => {
                // Bolalar ro'yxati (parser indentatsiyadan beradi yoki qo'lda list).
                for it in items {
                    children.push(it);
                }
            }
            // Matn bo'lishi mumkin bo'lgan skalyar qiymatlar.
            other @ (Value::Str(_)
            | Value::Int(_)
            | Value::Flt(_)
            | Value::Sym(_)
            | Value::Bool(_)) => {
                text = Some(other.to_text());
            }
            Value::Nil => {}
            other => {
                return Err(Flow::err(format!(
                    "{} elementi qo'llab-quvvatlamaydigan argument: {}",
                    tag,
                    other.type_name()
                )));
            }
        }
    }

    let mut node: BTreeMap<String, Value> = BTreeMap::new();
    node.insert("__node".to_string(), Value::Bool(true));
    node.insert("tag".to_string(), Value::Str(tag.to_string()));
    if let Some(t) = text {
        node.insert("text".to_string(), Value::Str(t));
    }
    if !props.is_empty() {
        node.insert("props".to_string(), Value::Map(props));
    }
    if !children.is_empty() {
        node.insert("children".to_string(), Value::List(children));
    }
    Ok(Value::Map(node))
}

// Qiymat element ({__node:true}) ekanmi.
fn is_node(v: &Value) -> bool {
    matches!(v, Value::Map(m) if matches!(m.get("__node"), Some(Value::Bool(true))))
}

// Public: interp view tanasidagi element qiymatlarini aniqlash uchun.
pub fn is_node_value(v: &Value) -> bool {
    is_node(v)
}

// Bir nechta top-level elementni ko'rinmas o'rovga (fragment) yig'adi. Fragment
// HTML'da yopuvchi tegsiz — faqat bolalari render qilinadi (React fragment kabi).
pub fn fragment(children: Vec<Value>) -> Value {
    let mut node: BTreeMap<String, Value> = BTreeMap::new();
    node.insert("__node".to_string(), Value::Bool(true));
    node.insert("tag".to_string(), Value::Str("__fragment".to_string()));
    node.insert("children".to_string(), Value::List(children));
    Value::Map(node)
}

// `ui.*` dispatch — Interp'ga ulanadi (kelajakda `ui.serve` state kerak).
// MVP'da faqat `ui.html`. eval_call shu yerga yo'naltiradi.
impl Interp {
    pub fn ui_dispatch(self: &Arc<Self>, func: &str, args: Vec<Value>) -> Result<Value, Flow> {
        match func {
            // ui.html node -> str (element/komponent server-side render). To'liq
            // hujjat EMAS — faqat element daraxti HTML'i (3-bosqich ui.serve to'liq
            // sahifani theme + body bilan birlashtiradi). Argument {__node} yoki nil.
            "html" => {
                let node = args.first().cloned().unwrap_or(Value::Nil);
                Ok(Value::Str(node_to_html(&node)))
            }
            // ui.css -> str: theme tokenlaridan CSS custom properties + base CSS.
            // `<style>` ichiga qo'yiladi (ui.serve yoki qo'lda).
            "css" => {
                let theme = self.theme.read();
                Ok(Value::Str(theme_to_css(&theme)))
            }
            // ui.page node -> str: to'liq HTML hujjat (doctype + head[theme css] +
            // body[node]). ui.serve shuni beradi; qo'lda render uchun ham foydali.
            "page" => {
                let node = args.first().cloned().unwrap_or(Value::Nil);
                let css = {
                    let theme = self.theme.read();
                    theme_to_css(&theme)
                };
                Ok(Value::Str(full_document(&css, &node_to_html(&node))))
            }
            _ => Err(Flow::err(format!("ui.{} funksiyasi yo'q", func))),
        }
    }
}

// theme tokenlarini CSS custom properties'ga aylantiradi + base semantik CSS.
//   theme {primary "#e84d8a" radius :lg}  ->  :root{--primary:#e84d8a;--radius:lg}
fn theme_to_css(theme: &BTreeMap<String, Value>) -> String {
    let mut out = String::new();
    out.push_str(":root{");
    for (k, v) in theme {
        out.push_str("--");
        out.push_str(k);
        out.push(':');
        out.push_str(&v.to_text());
        out.push(';');
    }
    out.push('}');
    // Semantik proplar uchun minimal base CSS (kind/pad/gap). To'liq dizayn
    // tizimi keyingi bosqich; hozir tokenlarni ishlatadigan asos.
    out.push_str(BASE_CSS);
    out
}

// Semantik prop class'lari uchun minimal base CSS. theme tokenlariga (`--primary`
// va h.k.) bog'lanadi, shunda `{kind::primary}` -> `.flux-primary` rang oladi.
const BASE_CSS: &str = "\
.flux-primary{background:var(--primary,#333);color:#fff}\
.flux-muted{color:var(--muted,#888)}\
.flux-panel{padding:1rem;border-radius:var(--radius,8px);background:var(--surface,#fff)}\
.flux-badge{display:inline-block;padding:.2em .5em;border-radius:.4em;font-size:.85em}";

// To'liq HTML hujjat: doctype + head (theme CSS) + body (element HTML).
fn full_document(css: &str, body_html: &str) -> String {
    format!(
        "<!doctype html><html><head><meta charset=\"utf-8\">\
<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\
<style>{}</style></head><body>{}</body></html>",
        css, body_html
    )
}

// --- SSR: element daraxti -> HTML string (sof funksiya) ---

// `{__node}` map'ni HTML stringga aylantiradi. Element bo'lmagan qiymat (matn)
// to'g'ridan-to'g'ri escape qilinib chiqadi. nil -> bo'sh string.
pub fn node_to_html(v: &Value) -> String {
    match v {
        Value::Nil => String::new(),
        Value::Map(m) if is_node(v) => {
            let tag = match m.get("tag") {
                Some(Value::Str(t)) => t.as_str(),
                _ => "div",
            };
            // Fragment — ko'rinmas o'rov: faqat bolalarni render qiladi (teg yo'q).
            if tag == "__fragment" {
                let mut out = String::new();
                if let Some(Value::List(items)) = m.get("children") {
                    for c in items {
                        out.push_str(&node_to_html(c));
                    }
                }
                return out;
            }
            let html_tag = html_tag_name(tag);
            let mut out = String::new();
            out.push('<');
            out.push_str(html_tag);
            out.push_str(&attrs_html(tag, m.get("props")));
            if is_void_tag(html_tag) {
                out.push_str(" />");
                return out;
            }
            out.push('>');
            // text bola (escape qilinadi).
            if let Some(Value::Str(t)) = m.get("text") {
                out.push_str(&escape_html(t));
            }
            // children (rekursiv render).
            if let Some(Value::List(items)) = m.get("children") {
                for c in items {
                    out.push_str(&node_to_html(c));
                }
            }
            out.push_str("</");
            out.push_str(html_tag);
            out.push('>');
            out
        }
        // Element bo'lmagan qiymat (matn/son) — escape qilingan matn.
        other => escape_html(&other.to_text()),
    }
}

// Flux teg nomini HTML teg nomiga moslaydi (semantik nomlar -> HTML).
fn html_tag_name(tag: &str) -> &str {
    match tag {
        "btn" => "button",
        "badge" => "span",
        other => other,
    }
}

// Yopilmaydigan (void) HTML teglari — bola/yopuvchi teg olmaydi.
fn is_void_tag(html_tag: &str) -> bool {
    matches!(html_tag, "img" | "input" | "br" | "hr")
}

// Props map'ni HTML atributlariga aylantiradi. MVP'da semantik proplar CSS
// class'ga aylanadi (`kind::primary pad:4` -> class="flux-primary flux-pad-4`),
// `id`/`href`/`src`/`placeholder`/`type`/`value`/`alt` esa to'g'ridan-to'g'ri
// HTML atributi bo'ladi. `on:`/`bind:` (event/binding) keyingi bosqich — MVP'da
// e'tiborsiz qoldiriladi (statik render).
fn attrs_html(tag: &str, props: Option<&Value>) -> String {
    let Some(Value::Map(p)) = props else {
        // `badge` semantik teg — base class beriladi.
        return base_class_attr(tag, &[]);
    };
    let mut classes: Vec<String> = Vec::new();
    let mut attrs: Vec<(String, String)> = Vec::new();
    for (k, v) in p {
        // event/binding proplari MVP'da statik renderda chiqarilmaydi.
        if k == "on" || k == "bind" {
            continue;
        }
        // To'g'ridan-to'g'ri HTML atributlari.
        if matches!(
            k.as_str(),
            "id" | "href" | "src" | "placeholder" | "type" | "value" | "alt" | "name" | "title"
        ) {
            attrs.push((k.clone(), v.to_text()));
            continue;
        }
        // Qolgani semantik prop -> CSS class `flux-<k>-<v>` yoki `flux-<v>`.
        match v {
            Value::Sym(s) => classes.push(format!("flux-{}", s)),
            Value::Bool(true) => classes.push(format!("flux-{}", k)),
            Value::Bool(false) | Value::Nil => {}
            other => classes.push(format!("flux-{}-{}", k, other.to_text())),
        }
    }
    let mut out = base_class_attr(tag, &classes);
    for (k, val) in attrs {
        out.push(' ');
        out.push_str(&escape_attr(&k));
        out.push_str("=\"");
        out.push_str(&escape_attr(&val));
        out.push('"');
    }
    out
}

// `badge` kabi semantik teglar uchun base class + qo'shimcha class'lar.
fn base_class_attr(tag: &str, extra: &[String]) -> String {
    let mut classes: Vec<String> = Vec::new();
    if tag == "badge" {
        classes.push("flux-badge".to_string());
    }
    classes.extend(extra.iter().cloned());
    if classes.is_empty() {
        return String::new();
    }
    format!(" class=\"{}\"", escape_attr(&classes.join(" ")))
}

// HTML matn kontekstida xavfli belgilarni escape qiladi.
fn escape_html(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            _ => out.push(c),
        }
    }
    out
}

// HTML atribut qiymati kontekstida escape (qo'shtirnoq ham).
fn escape_attr(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            _ => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(tag: &str, args: Vec<Value>) -> Value {
        // Flow Debug implement qilmaydi — .unwrap() o'rniga match.
        match build_node(tag, args) {
            Ok(v) => v,
            Err(_) => panic!("build_node xato qaytardi"),
        }
    }

    #[test]
    fn oddiy_matnli_element() {
        let n = node("h1", vec![Value::Str("Salom".into())]);
        assert_eq!(node_to_html(&n), "<h1>Salom</h1>");
    }

    #[test]
    fn matn_escape_qilinadi() {
        let n = node("p", vec![Value::Str("a < b & c".into())]);
        assert_eq!(node_to_html(&n), "<p>a &lt; b &amp; c</p>");
    }

    #[test]
    fn nested_children() {
        let inner = node("h1", vec![Value::Str("Sarlavha".into())]);
        let p = node("p", vec![Value::Str("matn".into())]);
        let outer = node("div", vec![Value::List(vec![inner, p])]);
        assert_eq!(
            node_to_html(&outer),
            "<div><h1>Sarlavha</h1><p>matn</p></div>"
        );
    }

    #[test]
    fn semantik_prop_class_boladi() {
        let mut props = BTreeMap::new();
        props.insert("kind".to_string(), Value::Sym("primary".into()));
        props.insert("pad".to_string(), Value::Int(4));
        let n = node("btn", vec![Value::Str("Saqlash".into()), Value::Map(props)]);
        let html = node_to_html(&n);
        // btn -> button, kind::primary -> flux-primary, pad:4 -> flux-pad-4
        assert!(html.starts_with("<button class=\""), "html: {}", html);
        assert!(html.contains("flux-primary"), "html: {}", html);
        assert!(html.contains("flux-pad-4"), "html: {}", html);
        assert!(html.contains(">Saqlash</button>"), "html: {}", html);
    }

    #[test]
    fn html_atribut_togridan() {
        let mut props = BTreeMap::new();
        props.insert("src".to_string(), Value::Str("/rasm.png".into()));
        props.insert("alt".to_string(), Value::Str("rasm".into()));
        let n = node("img", vec![Value::Map(props)]);
        let html = node_to_html(&n);
        // img — void teg
        assert!(html.starts_with("<img"), "html: {}", html);
        assert!(html.contains("src=\"/rasm.png\""), "html: {}", html);
        assert!(html.ends_with("/>"), "html: {}", html);
    }

    #[test]
    fn nil_bosh_string() {
        assert_eq!(node_to_html(&Value::Nil), "");
    }

    #[test]
    fn badge_base_class() {
        let n = node("badge", vec![Value::Str("Yangi".into())]);
        let html = node_to_html(&n);
        // badge -> span.flux-badge
        assert_eq!(html, "<span class=\"flux-badge\">Yangi</span>");
    }

    #[test]
    fn fragment_yopuvchi_tegsiz() {
        let a = node("h1", vec![Value::Str("A".into())]);
        let b = node("p", vec![Value::Str("B".into())]);
        let frag = fragment(vec![a, b]);
        // fragment teg chiqarmaydi — faqat bolalar.
        assert_eq!(node_to_html(&frag), "<h1>A</h1><p>B</p>");
    }

    #[test]
    fn theme_css_custom_properties() {
        let mut theme = BTreeMap::new();
        theme.insert("primary".to_string(), Value::Str("#e84d8a".into()));
        theme.insert("radius".to_string(), Value::Sym("lg".into()));
        let css = theme_to_css(&theme);
        // sym `:` prefiksisiz (to_text), str o'z holicha.
        assert!(css.contains("--primary:#e84d8a;"), "css: {}", css);
        assert!(css.contains("--radius:lg;"), "css: {}", css);
        assert!(css.contains(".flux-primary{"), "base css yo'q: {}", css);
    }
}

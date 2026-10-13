//! Pharmacy helpers.
//!
//! **Templates** (File → New → New from Template): each opens a new document at the right size and
//! fills it with ordinary, editable frames (coloured bands, headings and placeholder text), so
//! nothing about them is special once they are on the page.
//!
//! **Label Checklist** (`preflight.labelCheck`): scans every story for what a medicine label is
//! expected to carry (strength, pack size, batch, expiry, storage, the children warning, …) and for
//! fields left blank, and reports each item as found or missing. It is a study aid, not a
//! regulatory review.
//!
//! **References** (`text.references`): formats a numbered reference list (Vancouver or APA) from
//! citation records, Crossref works (DOI lookups) or PubMed summaries (PMID lookups), and places
//! it in a new text frame or at the text cursor. The lookups themselves happen in the app (they
//! need the network); this command only formats, so it works offline, on the web and in tests.

use serde_json::{Value, json};

use super::{CommandSpec, bad, cmd, str_param};
use crate::{Result, Session};

const MM: f64 = designcraft_geom::units::PT_PER_MM;

/// A template's id, menu name and one-line description (`file.templates` lists them).
pub struct TemplateInfo {
    pub id: &'static str,
    pub name: &'static str,
    pub description: &'static str,
    /// Page size in points.
    pub width: f64,
    pub height: f64,
}

pub const TEMPLATES: &[TemplateInfo] = &[
    TemplateInfo {
        id: "poster",
        name: "Research Poster",
        description: "A1 portrait: title band, authors, three columns from Introduction to References",
        width: 594.0 * MM,
        height: 841.0 * MM,
    },
    TemplateInfo {
        id: "leaflet",
        name: "Patient Leaflet",
        description: "A4 patient information leaflet with the six standard sections",
        width: 210.0 * MM,
        height: 297.0 * MM,
    },
    TemplateInfo {
        id: "label",
        name: "Bottle Label",
        description: "180 × 60 mm wrap-around label: composition, brand panel, warnings and batch",
        width: 180.0 * MM,
        height: 60.0 * MM,
    },
];

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(query "file.templates", "Templates", [], None, "{} → [{id, name, description, width, height}]", super::always, |_, _| {
            Ok(Value::Array(
                TEMPLATES
                    .iter()
                    .map(|t| json!({"id": t.id, "name": t.name, "description": t.description, "width": t.width, "height": t.height}))
                    .collect(),
            ))
        }),
        cmd!(noundo "file.newFromTemplate", "New from Template…", ["File", "New"], None,
            "{template: \"poster\"|\"leaflet\"|\"label\"} — a new document laid out for a pharmacy task → {index, frames}",
            super::always, new_from_template),
        cmd!(query "preflight.labelCheck", "Label Checklist…", ["Window", "Output"], None,
            "{} → {items: [{id, label, found: bool, hint}], blanks: [text before an empty field], missing: count} — what a medicine label is expected to carry, checked against the document's text",
            super::has_doc, label_check),
        cmd!(
            "text.references",
            "Insert References…",
            ["Type"],
            None,
            "{references: [record | Crossref work | PubMed summary | string (used as written)], style?: \"vancouver\"|\"apa\", title?: \"References\", rect?: [x0,y0,x1,y1] (a new frame; default: at the cursor, else a frame inside the first page's margins)} → {count, text, id?, story?}",
            super::has_doc,
            references
        ),
    ]
}

fn new_from_template(s: &mut Session, p: &Value) -> Result<Value> {
    let id = str_param(p, "template").ok_or_else(|| bad("file.newFromTemplate", "missing `template`"))?;
    let info = TEMPLATES.iter().find(|t| t.id == id).ok_or_else(|| {
        let ids: Vec<&str> = TEMPLATES.iter().map(|t| t.id).collect();
        bad("file.newFromTemplate", format!("unknown template `{id}` (one of {})", ids.join(", ")))
    })?;
    let margin = if info.id == "label" { 3.0 * MM } else { 15.0 * MM };
    let made = s.execute(
        "file.new",
        &json!({"width": info.width, "height": info.height, "pages": 1, "facingPages": false, "margins": margin, "bleed": 3.0 * MM, "title": info.name}),
    )?;
    let mut b = Builder { s, frames: 0 };
    match info.id {
        "poster" => poster(&mut b, info)?,
        "leaflet" => leaflet(&mut b, info)?,
        _ => label(&mut b, info)?,
    }
    let frames = b.frames;
    let _ = s.execute("edit.deselectAll", &json!({}));
    Ok(json!({"index": made.get("index").cloned().unwrap_or(Value::Null), "frames": frames}))
}

/// One Label Checklist item: an id, what to look for, and a regex over the label's text.
struct Check {
    id: &'static str,
    label: &'static str,
    hint: &'static str,
    pattern: &'static str,
}

const LABEL_CHECKS: &[Check] = &[
    Check { id: "strength", label: "Strength per unit", hint: "e.g. 500 mg per capsule", pattern: r"\d+(\.\d+)?\s?(mg|mcg|µg|g|ml|iu|%)\b" },
    Check {
        id: "packSize",
        label: "Pack size",
        hint: "e.g. 20 capsules, 100 ml",
        pattern: r"\d+\s?(capsules?|tablets?|sachets?|ampoules?|vials?|caplets?|lozenges?|ml)\b",
    },
    Check {
        id: "dosage",
        label: "Dosage / directions",
        hint: "how much, how often",
        pattern: r"dosage|\bdose\b|directions|as directed|times (a|per) day",
    },
    Check {
        id: "route",
        label: "Route of administration",
        hint: "e.g. for oral use",
        pattern: r"\boral|by mouth|swallow|external use|topical|inject|inhal|rectal|vaginal",
    },
    Check {
        id: "composition",
        label: "Composition / excipients",
        hint: "each capsule contains …",
        pattern: r"contains|composition|excipients?|active ingredients?",
    },
    Check { id: "batch", label: "Batch / lot number", hint: "Batch No. or Lot", pattern: r"\bbatch\b|\blot\b|b\.?\s?no\b" },
    Check { id: "mfg", label: "Manufacturing date", hint: "Mfg. Date", pattern: r"\bmfg\b|\bmfd\b|manufactur(e|ing) date|date of manufacture" },
    Check { id: "expiry", label: "Expiry date", hint: "Exp. Date", pattern: r"\bexp\b|expiry|expiration|use before|use by" },
    Check { id: "storage", label: "Storage conditions", hint: "e.g. store below 25 °C", pattern: r"\bstore\b|storage|°\s?c\b|refrigerat" },
    Check {
        id: "children",
        label: "Keep out of reach of children",
        hint: "standard safety warning",
        pattern: r"reach (and sight )?of children|away from children",
    },
    Check {
        id: "warnings",
        label: "Warnings / precautions",
        hint: "do not exceed the stated dose …",
        pattern: r"warning|caution|precaution|do not exceed|contraindicat",
    },
    Check { id: "registration", label: "Registration number", hint: "Reg. No.", pattern: r"\breg(\.|istration)?\s?(no|number)\b|registration" },
    Check {
        id: "manufacturer",
        label: "Manufacturer / marketing holder",
        hint: "made by …",
        pattern: r"manufactured by|made by|marketed by|marketing authori[sz]ation holder|\bltd\b|pharma",
    },
];

fn label_check(s: &mut Session, _: &Value) -> Result<Value> {
    let d = &s.doc()?.doc;
    let text: String = d.stories.values().map(|st| st.text.as_str()).collect::<Vec<_>>().join("\n");
    let mut items = Vec::new();
    let mut missing = 0usize;
    for c in LABEL_CHECKS {
        let re = regex::RegexBuilder::new(c.pattern).case_insensitive(true).build().map_err(|e| bad("preflight.labelCheck", e.to_string()))?;
        let found = re.is_match(&text);
        if !found {
            missing = missing.saturating_add(1);
        }
        items.push(json!({"id": c.id, "label": c.label, "found": found, "hint": c.hint}));
    }
    // A field name with nothing after it ("Exp. Date: " at the end of a line) was never filled in;
    // "…" is the templates' placeholder.
    let blank = regex::Regex::new(r"(?m)^\s*([^:\n]{2,40}):[ \t]*$").map_err(|e| bad("preflight.labelCheck", e.to_string()))?;
    let mut blanks: Vec<String> = blank.captures_iter(&text).filter_map(|c| c.get(1)).map(|m| m.as_str().trim().to_string()).collect();
    blanks.extend(text.lines().filter(|l| l.contains('…')).map(|l| l.trim().chars().take(60).collect::<String>()));
    blanks.dedup();
    blanks.truncate(50);
    Ok(json!({"items": items, "blanks": blanks, "missing": missing}))
}

/// A citation in the fields a reference list needs.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Reference {
    /// "Family Given-initials" pairs, in order.
    pub authors: Vec<(String, String)>,
    pub title: String,
    pub journal: String,
    pub year: String,
    pub volume: String,
    pub issue: String,
    pub pages: String,
    pub doi: String,
    pub pmid: String,
}

fn s_of(v: &Value) -> String {
    match v {
        Value::String(s) => s.trim().to_string(),
        Value::Number(n) => n.to_string(),
        Value::Array(a) => a.first().map(s_of).unwrap_or_default(),
        _ => String::new(),
    }
}

/// Initials from given names: "Jane Mary" → "JM", "J.-P." → "JP".
fn initials(given: &str) -> String {
    given.split([' ', '-', '.']).filter_map(|w| w.chars().next()).filter(|c| c.is_alphabetic()).flat_map(char::to_uppercase).collect()
}

impl Reference {
    /// Reads our own record shape, a Crossref work (`message` of /works/{doi}) or a PubMed
    /// esummary entry. Unknown fields are ignored; nothing here can fail.
    pub fn from_json(v: &Value) -> Self {
        let mut r = Reference::default();
        // Crossref: author [{given, family}], container-title [..], issued {date-parts [[y, m, d]]}.
        if let Some(a) = v.get("author").and_then(Value::as_array) {
            for au in a.iter().take(100) {
                let family = s_of(au.get("family").unwrap_or(&Value::Null));
                let given = s_of(au.get("given").unwrap_or(&Value::Null));
                let name = if family.is_empty() { s_of(au.get("name").unwrap_or(&Value::Null)) } else { family };
                if !name.is_empty() {
                    r.authors.push((name, initials(&given)));
                }
            }
        }
        // PubMed esummary: authors [{name: "Smith J"}].
        if let Some(a) = v.get("authors").and_then(Value::as_array) {
            for au in a.iter().take(100) {
                let full = match au {
                    Value::String(s) => s.clone(),
                    _ => s_of(au.get("name").unwrap_or(&Value::Null)),
                };
                let full = full.trim();
                if full.is_empty() {
                    continue;
                }
                match full.rsplit_once(' ') {
                    Some((fam, ini)) if ini.chars().all(|c| c.is_uppercase()) => r.authors.push((fam.to_string(), ini.to_string())),
                    _ => r.authors.push((full.to_string(), String::new())),
                }
            }
        }
        let get = |k: &str| v.get(k).map(s_of).unwrap_or_default();
        r.title = get("title").trim_end_matches('.').to_string();
        r.journal = [get("container-title-short"), get("container-title"), get("source"), get("journal")]
            .into_iter()
            .find(|s| !s.is_empty())
            .unwrap_or_default();
        r.year = v
            .pointer("/issued/date-parts/0/0")
            .or_else(|| v.pointer("/published/date-parts/0/0"))
            .map(s_of)
            .filter(|y| !y.is_empty())
            .unwrap_or_else(|| {
                let y = [get("year"), get("pubdate")].into_iter().find(|s| !s.is_empty()).unwrap_or_default();
                y.chars().take(4).collect()
            });
        r.volume = get("volume");
        r.issue = get("issue");
        r.pages = [get("page"), get("pages")].into_iter().find(|s| !s.is_empty()).unwrap_or_default();
        r.doi = [get("DOI"), get("doi")].into_iter().find(|s| !s.is_empty()).unwrap_or_default();
        if r.doi.is_empty()
            && let Some(ids) = v.get("articleids").and_then(Value::as_array)
        {
            r.doi = ids.iter().find(|i| i.get("idtype").and_then(Value::as_str) == Some("doi")).map(|i| s_of(&i["value"])).unwrap_or_default();
        }
        r.pmid = [get("pmid"), get("uid")].into_iter().find(|s| !s.is_empty()).unwrap_or_default();
        r
    }

    /// Vancouver (ICMJE): up to six authors then "et al.".
    pub fn vancouver(&self) -> String {
        let mut names: Vec<String> = self.authors.iter().take(6).map(|(f, i)| if i.is_empty() { f.clone() } else { format!("{f} {i}") }).collect();
        if self.authors.len() > 6 {
            names.push("et al".into());
        }
        let mut out = String::new();
        if !names.is_empty() {
            out.push_str(&names.join(", "));
            out.push_str(". ");
        }
        if !self.title.is_empty() {
            out.push_str(&self.title);
            out.push_str(". ");
        }
        if !self.journal.is_empty() {
            out.push_str(&self.journal);
            out.push_str(". ");
        }
        out.push_str(&self.year);
        if !self.volume.is_empty() {
            out.push(';');
            out.push_str(&self.volume);
            if !self.issue.is_empty() {
                out.push_str(&format!("({})", self.issue));
            }
        }
        if !self.pages.is_empty() {
            out.push(':');
            out.push_str(&self.pages);
        }
        out.push('.');
        if !self.doi.is_empty() {
            out.push_str(&format!(" doi:{}", self.doi));
        } else if !self.pmid.is_empty() {
            out.push_str(&format!(" PMID: {}", self.pmid));
        }
        out
    }

    /// APA 7: "Family, I. I., & Family, I. (Year). Title. Journal, Volume(Issue), pages. https://doi.org/…".
    pub fn apa(&self) -> String {
        let fmt = |(f, i): &(String, String)| {
            if i.is_empty() {
                f.clone()
            } else {
                let dotted: Vec<String> = i.chars().map(|c| format!("{c}.")).collect();
                format!("{f}, {}", dotted.join(" "))
            }
        };
        let names: Vec<String> = self.authors.iter().take(20).map(fmt).collect();
        let authors = match names.len() {
            0 => String::new(),
            1 => names[0].clone(),
            n => format!("{}, & {}", names[..n - 1].join(", "), names[n - 1]),
        };
        let mut out = String::new();
        if !authors.is_empty() {
            out.push_str(&authors);
            out.push(' ');
        }
        out.push_str(&format!("({}). ", if self.year.is_empty() { "n.d." } else { &self.year }));
        if !self.title.is_empty() {
            out.push_str(&self.title);
            out.push_str(". ");
        }
        if !self.journal.is_empty() {
            out.push_str(&self.journal);
            if !self.volume.is_empty() {
                out.push_str(&format!(", {}", self.volume));
                if !self.issue.is_empty() {
                    out.push_str(&format!("({})", self.issue));
                }
            }
            if !self.pages.is_empty() {
                out.push_str(&format!(", {}", self.pages));
            }
            out.push_str(". ");
        }
        if !self.doi.is_empty() {
            out.push_str(&format!("https://doi.org/{}", self.doi));
        }
        out.trim_end().to_string()
    }
}

/// The formatted list: Vancouver numbered in citation order, APA alphabetical by first author.
pub fn format_references(refs: &[Value], style: &str) -> Vec<String> {
    let apa = style.eq_ignore_ascii_case("apa");
    let mut lines: Vec<String> = refs
        .iter()
        .take(500)
        .filter_map(|v| match v {
            Value::String(s) if !s.trim().is_empty() => Some(s.trim().to_string()),
            Value::Object(_) => {
                let r = Reference::from_json(v);
                Some(if apa { r.apa() } else { r.vancouver() })
            }
            _ => None,
        })
        .collect();
    if apa {
        lines.sort_by_key(|l| l.to_lowercase());
        lines
    } else {
        lines.into_iter().enumerate().map(|(i, l)| format!("{}. {l}", i.saturating_add(1))).collect()
    }
}

fn references(s: &mut Session, p: &Value) -> Result<Value> {
    let refs = p.get("references").and_then(Value::as_array).ok_or_else(|| bad("text.references", "missing `references` (an array)"))?;
    let style = str_param(p, "style").unwrap_or("vancouver");
    let lines = format_references(refs, style);
    if lines.is_empty() {
        return Err(bad("text.references", "no references to insert"));
    }
    let title = str_param(p, "title").unwrap_or("References");
    let body = lines.join("\n");
    let text = if title.is_empty() { body.clone() } else { format!("{title}\n{body}") };
    let rect = super::rect_param(p, "rect");
    // At the text cursor when there is one and no frame was asked for.
    if rect.is_none() && super::has_text(s).is_ok() {
        s.execute("text.insert", &json!({"text": body, "raw": true}))?;
        return Ok(json!({"count": lines.len(), "text": body}));
    }
    let rect = match rect {
        Some(r) => [r.x0, r.y0, r.x1, r.y1],
        None => {
            let d = &s.doc()?.doc;
            let page = d.spreads.first().and_then(|sp| sp.pages.first()).ok_or_else(|| bad("text.references", "the document has no pages"))?;
            let m = page.margins;
            let (x, w, h) = (page.x, page.width, page.height);
            [x + m.inside, (h * 0.55).max(m.top), (x + w - m.outside).max(x + m.inside + 36.0), (h - m.bottom).max(h * 0.55 + 36.0)]
        }
    };
    let made = s.execute("frame.create", &json!({"rect": rect, "content": "text", "text": text, "caret": false}))?;
    s.execute("type.char", &json!({"attrs": {"size": 9.0}}))?;
    s.execute("type.para", &json!({"attrs": {"spaceAfter": 3.0}}))?;
    Ok(json!({"count": lines.len(), "text": text, "id": made["id"], "story": made["story"]}))
}

/// Places frames through the ordinary commands.
struct Builder<'a> {
    s: &'a mut Session,
    frames: usize,
}

/// Text formatting for [`Builder::text`].
#[derive(Clone, Copy)]
struct Txt<'a> {
    size: f64,
    style: &'a str,
    color: &'a str,
    align: &'a str,
    family: &'a str,
}

const SANS: &str = "Source Sans 3";

impl Txt<'static> {
    fn body(size: f64) -> Self {
        Txt { size, style: "Regular", color: "Ink", align: "left", family: SANS }
    }
    fn heading(size: f64) -> Self {
        Txt { size, style: "Bold", color: "Brand", align: "left", family: SANS }
    }
}

impl<'a> Txt<'a> {
    fn color(mut self, c: &'a str) -> Self {
        self.color = c;
        self
    }
    fn center(mut self) -> Self {
        self.align = "center";
        self
    }
    fn style(mut self, s: &'a str) -> Self {
        self.style = s;
        self
    }
}

impl Builder<'_> {
    fn swatch(&mut self, name: &str, hex: &str) -> Result<()> {
        self.s.execute("swatch.create", &json!({"name": name, "color": hex}))?;
        Ok(())
    }

    /// A filled, unstroked box.
    fn band(&mut self, rect: [f64; 4], swatch: &str) -> Result<()> {
        let id = self.s.execute("frame.create", &json!({"rect": rect, "content": "unassigned"}))?.get("id").cloned().unwrap_or(Value::Null);
        self.s.execute("object.fill", &json!({"swatch": swatch, "ids": [id]}))?;
        self.s.execute("object.stroke", &json!({"swatch": "[None]", "ids": [id]}))?;
        self.frames = self.frames.saturating_add(1);
        Ok(())
    }

    fn text(&mut self, rect: [f64; 4], text: &str, t: Txt<'_>) -> Result<()> {
        self.s.execute("frame.create", &json!({"rect": rect, "content": "text", "text": text, "caret": false}))?;
        self.s.execute("type.char", &json!({"attrs": {"fontFamily": t.family, "fontStyle": t.style, "size": t.size, "fill": t.color}}))?;
        self.s.execute("type.para", &json!({"attrs": {"align": t.align, "spaceAfter": t.size * 0.4}}))?;
        self.frames = self.frames.saturating_add(1);
        Ok(())
    }
}

fn palette(b: &mut Builder<'_>) -> Result<()> {
    b.swatch("Brand", "#1b2a4a")?;
    b.swatch("Accent", "#f28c28")?;
    b.swatch("Sky", "#d4eff9")?;
    b.swatch("Ink", "#222222")?;
    b.swatch("Muted", "#6b7280")
}

fn poster(b: &mut Builder<'_>, info: &TemplateInfo) -> Result<()> {
    palette(b)?;
    let (w, h, m) = (info.width, info.height, 15.0 * MM);
    b.band([0.0, 0.0, w, 150.0 * MM], "Brand")?;
    b.band([0.0, 150.0 * MM, w, 154.0 * MM], "Accent")?;
    b.text([m, 15.0 * MM, w - m, 100.0 * MM], "Your Poster Title Goes Here", Txt::heading(84.0).color("[Paper]").center())?;
    b.text(
        [m, 102.0 * MM, w - m, 145.0 * MM],
        "Author One, Author Two, Supervisor Name\nFaculty of Pharmacy and Biotechnology, German University in Cairo",
        Txt::body(40.0).color("[Paper]").center(),
    )?;
    let top = 170.0 * MM;
    let gutter = 10.0 * MM;
    let col = (w - 2.0 * m - 2.0 * gutter) / 3.0;
    let x = |i: f64| m + i * (col + gutter);
    let sections: [(f64, f64, f64, &str); 6] = [
        (0.0, top, 300.0 * MM, "Introduction"),
        (0.0, 310.0 * MM, h - 40.0 * MM, "Aim"),
        (1.0, top, 450.0 * MM, "Methods"),
        (1.0, 460.0 * MM, h - 40.0 * MM, "Results"),
        (2.0, top, 500.0 * MM, "Discussion"),
        (2.0, 510.0 * MM, h - 40.0 * MM, "Conclusion & References"),
    ];
    for (i, y0, y1, title) in sections {
        b.band([x(i), y0, x(i) + col, y0 + 24.0 * MM], "Sky")?;
        b.text([x(i) + 5.0 * MM, y0 + 3.0 * MM, x(i) + col - 5.0 * MM, y0 + 24.0 * MM], title, Txt::heading(40.0))?;
        b.text(
            [x(i) + 5.0 * MM, y0 + 28.0 * MM, x(i) + col - 5.0 * MM, y1],
            "Replace this text with your content. Keep each section short: 3–5 bullet points or a figure with a one-line caption reads best from 1.5 m away.",
            Txt::body(28.0),
        )?;
    }
    b.band([0.0, h - 30.0 * MM, w, h], "Brand")?;
    b.text([m, h - 22.0 * MM, w - m, h - 8.0 * MM], "Contact: name@student.guc.edu.eg", Txt::body(28.0).color("[Paper]").center())
}

fn leaflet(b: &mut Builder<'_>, info: &TemplateInfo) -> Result<()> {
    palette(b)?;
    let (w, h, m) = (info.width, info.height, 15.0 * MM);
    b.band([0.0, 0.0, w, 38.0 * MM], "Brand")?;
    b.text([m, 8.0 * MM, w - m, 20.0 * MM], "Package leaflet: Information for the patient", Txt::body(11.0).color("[Paper]"))?;
    b.text([m, 18.0 * MM, w - m, 34.0 * MM], "Product Name 500 mg capsules", Txt::heading(22.0).color("[Paper]"))?;
    b.text(
        [m, 44.0 * MM, w - m, 62.0 * MM],
        "Read all of this leaflet carefully before you start taking this medicine because it contains important information for you. Keep this leaflet. You may need to read it again.",
        Txt::body(9.0).style("Semibold"),
    )?;
    let sections = [
        "1. What this medicine is and what it is used for",
        "2. What you need to know before you take it",
        "3. How to take it",
        "4. Possible side effects",
        "5. How to store it",
        "6. Contents of the pack and other information",
    ];
    let gutter = 8.0 * MM;
    let col = (w - 2.0 * m - gutter) / 2.0;
    let row_h = (h - 70.0 * MM - 14.0 * MM) / 3.0;
    for (k, title) in sections.iter().enumerate() {
        let (c, r) = ((k / 3) as f64, (k % 3) as f64);
        let x0 = m + c * (col + gutter);
        let y0 = 68.0 * MM + r * row_h;
        b.band([x0, y0, x0 + col, y0 + 0.8 * MM], "Accent")?;
        b.text([x0, y0 + 2.0 * MM, x0 + col, y0 + 10.0 * MM], title, Txt::heading(10.5))?;
        b.text([x0, y0 + 11.0 * MM, x0 + col, y0 + row_h - 3.0 * MM], "Type the section text here.", Txt::body(9.0))?;
    }
    b.text([m, h - 12.0 * MM, w - m, h - 6.0 * MM], "This leaflet was last revised in: month YYYY", Txt::body(8.0).color("Muted"))
}

fn label(b: &mut Builder<'_>, info: &TemplateInfo) -> Result<()> {
    palette(b)?;
    let (w, h) = (info.width, info.height);
    let panel = w / 3.0;
    let pad = 3.0 * MM;
    // Centre (front) panel.
    b.band([panel, 0.0, 2.0 * panel, h], "Brand")?;
    b.band([panel, h - 7.0 * MM, 2.0 * panel, h - 5.5 * MM], "Accent")?;
    b.text([panel + pad, 8.0 * MM, 2.0 * panel - pad, 22.0 * MM], "Brand Name", Txt::heading(22.0).color("[Paper]").center())?;
    b.text(
        [panel + pad, 23.0 * MM, 2.0 * panel - pad, 38.0 * MM],
        "Active ingredient(s) and strength\nper capsule",
        Txt::body(8.5).color("[Paper]").center(),
    )?;
    b.text([panel + pad, 40.0 * MM, 2.0 * panel - pad, 50.0 * MM], "20 capsules", Txt::heading(11.0).color("Accent").center())?;
    // Left panel.
    b.text([pad, pad, panel - pad, 10.0 * MM], "Composition", Txt::heading(8.0))?;
    b.text(
        [pad, 10.0 * MM, panel - pad, h - pad],
        "Each capsule contains: …\nExcipients: …\n\nIndications: …\n\nDosage: as directed by the physician.\nFor oral use.",
        Txt::body(6.0),
    )?;
    // Right panel.
    b.text([2.0 * panel + pad, pad, w - pad, 10.0 * MM], "Warnings", Txt::heading(8.0))?;
    b.text(
        [2.0 * panel + pad, 10.0 * MM, w - pad, 32.0 * MM],
        "Keep out of the reach and sight of children.\nDo not exceed the stated dose.\nStore below 25 °C in a dry place.\nManufactured by: …",
        Txt::body(6.0),
    )?;
    b.band([2.0 * panel + pad, 33.0 * MM, w - pad, h - pad], "Sky")?;
    b.text(
        [2.0 * panel + 2.0 * pad, 35.0 * MM, w - 2.0 * pad, h - pad - 1.0 * MM],
        "Batch No.: \nMfg. Date: \nExp. Date: \nReg. No.: ",
        Txt::body(6.5).style("Semibold"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_template_builds_a_one_page_document() {
        for t in TEMPLATES {
            let mut s = Session::new();
            let r = s.execute("file.newFromTemplate", &json!({"template": t.id})).unwrap();
            assert!(r["frames"].as_u64().unwrap() >= 8, "{}: {r}", t.id);
            let d = s.doc().unwrap();
            assert_eq!(d.doc.spreads.len(), 1, "{}", t.id);
            assert!(d.doc.swatch("Brand").is_some());
        }
    }

    #[test]
    fn the_label_template_passes_most_checks_but_flags_its_blank_fields() {
        let mut s = Session::new();
        s.execute("file.newFromTemplate", &json!({"template": "label"})).unwrap();
        let r = s.execute("preflight.labelCheck", &json!({})).unwrap();
        let found = |id: &str| r["items"].as_array().unwrap().iter().find(|i| i["id"] == id).unwrap()["found"] == true;
        for id in ["batch", "expiry", "storage", "children", "packSize", "mfg"] {
            assert!(found(id), "{id}: {r}");
        }
        let blanks: Vec<&str> = r["blanks"].as_array().unwrap().iter().filter_map(Value::as_str).collect();
        assert!(blanks.iter().any(|b| b.starts_with("Batch No")), "{blanks:?}");
        assert!(blanks.iter().any(|b| b.starts_with("Exp")), "{blanks:?}");
    }

    #[test]
    fn an_empty_document_misses_everything_without_crashing() {
        let mut s = Session::new();
        s.execute("file.new", &json!({})).unwrap();
        let r = s.execute("preflight.labelCheck", &json!({})).unwrap();
        assert_eq!(r["missing"].as_u64().unwrap() as usize, LABEL_CHECKS.len());
    }

    #[test]
    fn every_check_pattern_compiles() {
        for c in LABEL_CHECKS {
            assert!(regex::Regex::new(c.pattern).is_ok(), "{}", c.id);
        }
    }

    #[test]
    fn a_crossref_work_formats_in_vancouver_and_apa() {
        let work = json!({
            "author": [{"given": "Fatma", "family": "Illangakoon"}, {"given": "Mark J.", "family": "Smith"}],
            "title": ["Fast dissolving paracetamol/caffeine nanofibers"],
            "container-title": ["International Journal of Pharmaceutics"],
            "container-title-short": ["Int J Pharm"],
            "issued": {"date-parts": [[2014, 5]]},
            "volume": "477", "issue": "1-2", "page": "369-379", "DOI": "10.1016/j.ijpharm.2014.10.036"
        });
        let r = Reference::from_json(&work);
        assert_eq!(
            r.vancouver(),
            "Illangakoon F, Smith MJ. Fast dissolving paracetamol/caffeine nanofibers. Int J Pharm. 2014;477(1-2):369-379. doi:10.1016/j.ijpharm.2014.10.036"
        );
        assert_eq!(
            r.apa(),
            "Illangakoon, F., & Smith, M. J. (2014). Fast dissolving paracetamol/caffeine nanofibers. Int J Pharm, 477(1-2), 369-379. https://doi.org/10.1016/j.ijpharm.2014.10.036"
        );
    }

    #[test]
    fn a_pubmed_summary_and_plain_text_both_format() {
        let pm = json!({"uid": "123", "authors": [{"name": "Smith J"}, {"name": "Doe AB"}], "title": "A title.", "source": "J Pharm Sci", "pubdate": "2020 Jan", "volume": "9", "pages": "1-5"});
        let lines = format_references(&[pm, json!("Already formatted reference."), json!(""), json!(3)], "vancouver");
        assert_eq!(
            lines,
            vec!["1. Smith J, Doe AB. A title. J Pharm Sci. 2020;9:1-5. PMID: 123".to_string(), "2. Already formatted reference.".to_string()]
        );
    }

    #[test]
    fn many_authors_become_et_al_and_hostile_input_does_not_crash() {
        let authors: Vec<Value> = (0..9).map(|i| json!({"family": format!("A{i}"), "given": "B"})).collect();
        let r = Reference::from_json(&json!({"author": authors, "title": 5, "issued": {"date-parts": "x"}}));
        assert!(r.vancouver().starts_with("A0 B, A1 B, A2 B, A3 B, A4 B, A5 B, et al. 5."), "{}", r.vancouver());
        let _ = Reference::from_json(&json!(null)).apa();
        let _ = Reference::from_json(&json!({"author": [{"given": "é-ü"}], "authors": ["", 7]})).vancouver();
    }

    #[test]
    fn references_land_in_a_new_frame() {
        let mut s = Session::new();
        s.execute("file.new", &json!({})).unwrap();
        let r = s.execute("text.references", &json!({"references": ["One.", "Two."], "style": "vancouver"})).unwrap();
        assert_eq!(r["count"], 2);
        assert!(r["text"].as_str().unwrap().starts_with("References\n1. One.\n2. Two."));
        assert!(s.execute("text.references", &json!({"references": []})).is_err());
        assert!(s.execute("text.references", &json!({})).is_err());
    }

    #[test]
    fn an_unknown_template_is_an_error_not_a_crash() {
        let mut s = Session::new();
        assert!(s.execute("file.newFromTemplate", &json!({"template": "nope"})).is_err());
        assert!(s.execute("file.newFromTemplate", &json!({})).is_err());
        assert_eq!(s.execute("file.templates", &json!({})).unwrap().as_array().unwrap().len(), TEMPLATES.len());
    }
}

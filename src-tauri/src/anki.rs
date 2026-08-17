//! Anki `.apkg` export for flashcard decks. An `.apkg` is a ZIP archive holding
//! a `collection.anki2` (a SQLite DB in Anki's schema) plus a `media` JSON map
//! (empty here — text-only cards). We build the collection in a temp SQLite file,
//! then pack it into a STORED (uncompressed) ZIP by hand, so no new crates are
//! pulled in. The legacy schema (`ver = 11`) imports cleanly into current Anki.

use crate::db::now_ms;
use crate::error::{Error, Result};
use rusqlite::{params, Connection};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

/// Process-unique token for temp filenames. `now_ms()` alone collides when two
/// exports/imports run in the same millisecond (e.g. parallel tests, or two
/// concurrent imports) — the loser then opens the other's half-written DB. Pair
/// the timestamp with a monotonic counter so every temp path is unique.
fn temp_token() -> String {
    static SEQ: AtomicU64 = AtomicU64::new(0);
    format!("{}-{}", now_ms(), SEQ.fetch_add(1, Ordering::Relaxed))
}

// ---- SHA-1 (for Anki's note checksum `csum`) --------------------------------

fn sha1(data: &[u8]) -> [u8; 20] {
    let mut h: [u32; 5] = [0x67452301, 0xEFCDAB89, 0x98BADCFE, 0x10325476, 0xC3D2E1F0];
    let ml = (data.len() as u64) * 8;
    let mut msg = data.to_vec();
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&ml.to_be_bytes());

    for chunk in msg.chunks_exact(64) {
        let mut w = [0u32; 80];
        for (i, wi) in w.iter_mut().take(16).enumerate() {
            *wi = u32::from_be_bytes([chunk[i * 4], chunk[i * 4 + 1], chunk[i * 4 + 2], chunk[i * 4 + 3]]);
        }
        for i in 16..80 {
            w[i] = (w[i - 3] ^ w[i - 8] ^ w[i - 14] ^ w[i - 16]).rotate_left(1);
        }
        let (mut a, mut b, mut c, mut d, mut e) = (h[0], h[1], h[2], h[3], h[4]);
        for (i, &wi) in w.iter().enumerate() {
            let (f, k) = match i {
                0..=19 => ((b & c) | ((!b) & d), 0x5A827999u32),
                20..=39 => (b ^ c ^ d, 0x6ED9EBA1),
                40..=59 => ((b & c) | (b & d) | (c & d), 0x8F1BBCDC),
                _ => (b ^ c ^ d, 0xCA62C1D6),
            };
            let tmp = a
                .rotate_left(5)
                .wrapping_add(f)
                .wrapping_add(e)
                .wrapping_add(k)
                .wrapping_add(wi);
            e = d;
            d = c;
            c = b.rotate_left(30);
            b = a;
            a = tmp;
        }
        h[0] = h[0].wrapping_add(a);
        h[1] = h[1].wrapping_add(b);
        h[2] = h[2].wrapping_add(c);
        h[3] = h[3].wrapping_add(d);
        h[4] = h[4].wrapping_add(e);
    }
    let mut out = [0u8; 20];
    for (i, hi) in h.iter().enumerate() {
        out[i * 4..i * 4 + 4].copy_from_slice(&hi.to_be_bytes());
    }
    out
}

/// Anki field checksum: the integer of the first 8 hex digits (first 4 bytes) of
/// the SHA-1 of the (HTML-stripped) first field. Used for duplicate detection.
fn field_checksum(field: &str) -> i64 {
    let stripped = strip_html(field);
    let d = sha1(stripped.as_bytes());
    u32::from_be_bytes([d[0], d[1], d[2], d[3]]) as i64
}

/// Stable Anki note GUID for one logical Cortex card. Anki uses note identity
/// while importing a package, so keeping this deterministic lets a repeated
/// export of the same material/front update the same logical note instead of
/// looking like a brand-new random card every time.
fn note_guid(identity: &str, front: &str) -> String {
    const ALPHABET: &[u8] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz";
    let source = format!("cortex\u{001f}{identity}\u{001f}{}", strip_html(front));
    let digest = sha1(source.as_bytes());
    let mut guid = String::with_capacity(10);
    for (idx, byte) in digest.iter().take(10).enumerate() {
        guid.push(ALPHABET[(*byte as usize + idx * 17) % ALPHABET.len()] as char);
    }
    guid
}

/// Minimal HTML tag strip — fronts are usually plain text, but Anki computes the
/// checksum on stripped content, so mirror that for stable dedupe.
fn strip_html(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut in_tag = false;
    for ch in s.chars() {
        match ch {
            '<' => in_tag = true,
            '>' => in_tag = false,
            c if !in_tag => out.push(c),
            _ => {}
        }
    }
    out
}

// ---- CRC32 (for the ZIP entries) --------------------------------------------

fn crc32(data: &[u8]) -> u32 {
    let mut crc: u32 = 0xFFFF_FFFF;
    for &byte in data {
        crc ^= byte as u32;
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xEDB8_8320 & mask);
        }
    }
    !crc
}

// ---- minimal STORED ZIP writer ----------------------------------------------

/// Pack named entries into a ZIP using the STORED (no-compression) method.
fn zip_store(entries: &[(&str, Vec<u8>)]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut central = Vec::new();
    let mut offsets: Vec<u32> = Vec::new();

    for (name, data) in entries {
        offsets.push(out.len() as u32);
        let crc = crc32(data);
        let nlen = name.len() as u16;
        let sz = data.len() as u32;
        // local file header
        out.extend_from_slice(&0x0403_4b50u32.to_le_bytes());
        out.extend_from_slice(&20u16.to_le_bytes()); // version needed
        out.extend_from_slice(&0u16.to_le_bytes()); // flags
        out.extend_from_slice(&0u16.to_le_bytes()); // method = stored
        out.extend_from_slice(&0u16.to_le_bytes()); // mod time
        out.extend_from_slice(&0u16.to_le_bytes()); // mod date
        out.extend_from_slice(&crc.to_le_bytes());
        out.extend_from_slice(&sz.to_le_bytes()); // compressed size
        out.extend_from_slice(&sz.to_le_bytes()); // uncompressed size
        out.extend_from_slice(&nlen.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes()); // extra len
        out.extend_from_slice(name.as_bytes());
        out.extend_from_slice(data);
    }

    for (idx, (name, data)) in entries.iter().enumerate() {
        let crc = crc32(data);
        let nlen = name.len() as u16;
        let sz = data.len() as u32;
        central.extend_from_slice(&0x0201_4b50u32.to_le_bytes());
        central.extend_from_slice(&20u16.to_le_bytes()); // version made by
        central.extend_from_slice(&20u16.to_le_bytes()); // version needed
        central.extend_from_slice(&0u16.to_le_bytes()); // flags
        central.extend_from_slice(&0u16.to_le_bytes()); // method
        central.extend_from_slice(&0u16.to_le_bytes()); // mod time
        central.extend_from_slice(&0u16.to_le_bytes()); // mod date
        central.extend_from_slice(&crc.to_le_bytes());
        central.extend_from_slice(&sz.to_le_bytes());
        central.extend_from_slice(&sz.to_le_bytes());
        central.extend_from_slice(&nlen.to_le_bytes());
        central.extend_from_slice(&0u16.to_le_bytes()); // extra len
        central.extend_from_slice(&0u16.to_le_bytes()); // comment len
        central.extend_from_slice(&0u16.to_le_bytes()); // disk start
        central.extend_from_slice(&0u16.to_le_bytes()); // internal attrs
        central.extend_from_slice(&0u32.to_le_bytes()); // external attrs
        central.extend_from_slice(&offsets[idx].to_le_bytes());
        central.extend_from_slice(name.as_bytes());
    }

    let cd_offset = out.len() as u32;
    let cd_size = central.len() as u32;
    out.extend_from_slice(&central);
    // end of central directory
    out.extend_from_slice(&0x0605_4b50u32.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes()); // disk num
    out.extend_from_slice(&0u16.to_le_bytes()); // disk w/ cd
    out.extend_from_slice(&(entries.len() as u16).to_le_bytes());
    out.extend_from_slice(&(entries.len() as u16).to_le_bytes());
    out.extend_from_slice(&cd_size.to_le_bytes());
    out.extend_from_slice(&cd_offset.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes()); // comment len
    out
}

// ---- collection.anki2 builder -----------------------------------------------

const COL_SCHEMA: &str = "
CREATE TABLE col (id integer PRIMARY KEY, crt integer NOT NULL, mod integer NOT NULL,
  scm integer NOT NULL, ver integer NOT NULL, dty integer NOT NULL, usn integer NOT NULL,
  ls integer NOT NULL, conf text NOT NULL, models text NOT NULL, decks text NOT NULL,
  dconf text NOT NULL, tags text NOT NULL);
CREATE TABLE notes (id integer PRIMARY KEY, guid text NOT NULL, mid integer NOT NULL,
  mod integer NOT NULL, usn integer NOT NULL, tags text NOT NULL, flds text NOT NULL,
  sfld text NOT NULL, csum integer NOT NULL, flags integer NOT NULL, data text NOT NULL);
CREATE TABLE cards (id integer PRIMARY KEY, nid integer NOT NULL, did integer NOT NULL,
  ord integer NOT NULL, mod integer NOT NULL, usn integer NOT NULL, type integer NOT NULL,
  queue integer NOT NULL, due integer NOT NULL, ivl integer NOT NULL, factor integer NOT NULL,
  reps integer NOT NULL, lapses integer NOT NULL, left integer NOT NULL, odue integer NOT NULL,
  odid integer NOT NULL, flags integer NOT NULL, data text NOT NULL);
CREATE TABLE revlog (id integer PRIMARY KEY, cid integer NOT NULL, usn integer NOT NULL,
  ease integer NOT NULL, ivl integer NOT NULL, lastIvl integer NOT NULL, factor integer NOT NULL,
  time integer NOT NULL, type integer NOT NULL);
CREATE TABLE graves (usn integer NOT NULL, oid integer NOT NULL, type integer NOT NULL);
CREATE INDEX ix_notes_csum on notes (csum);
CREATE INDEX ix_cards_nid on cards (nid);
CREATE INDEX ix_cards_sched on cards (did, queue, due);
CREATE INDEX ix_revlog_cid on revlog (cid);
";

#[derive(Clone, Copy)]
struct AnkiTemplate {
    name: &'static str,
    css: &'static str,
    qfmt: &'static str,
    afmt: &'static str,
}

const BASIC_TEMPLATE: AnkiTemplate = AnkiTemplate {
    name: "Cortex Basic",
    css: ".card{font-family:arial;font-size:20px;text-align:center;color:black;background:white;}",
    qfmt: "{{Front}}",
    afmt: "{{FrontSide}}\n\n<hr id=answer>\n\n{{Back}}",
};

// Anki cards are web pages, so this template can attach a small, dependency-free
// click handler to the options exported in a Cortex quiz. It checks the choice on
// the front immediately; the native Anki answer/rating controls still decide the
// spaced-repetition grade afterward.
const MULTIPLE_CHOICE_TEMPLATE: AnkiTemplate = AnkiTemplate {
    name: "Cortex Multiple Choice",
    css: r#"
.card { font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif; font-size: 20px; text-align: left; color: #1f2937; background: #ffffff; }
.cortex-mcq { max-width: 720px; margin: 0 auto; line-height: 1.45; }
.cortex-question { margin: 0 0 20px; font-size: 1.25em; font-weight: 650; }
.cortex-options { display: grid; gap: 10px; }
.cortex-choice { width: 100%; display: flex; align-items: flex-start; gap: 11px; padding: 13px 15px; color: inherit; background: #f8fafc; border: 1px solid #cbd5e1; border-radius: 10px; font: inherit; text-align: left; cursor: pointer; }
.cortex-choice:hover:not(:disabled) { background: #eef2ff; border-color: #6366f1; }
.cortex-choice:disabled { cursor: default; opacity: 1; }
.cortex-choice-key { flex: none; display: inline-grid; place-items: center; width: 1.65em; height: 1.65em; border: 1px solid currentColor; border-radius: 50%; font-size: .82em; font-weight: 700; }
.cortex-choice.cortex-correct { color: #166534; background: #dcfce7; border-color: #22c55e; }
.cortex-choice.cortex-wrong { color: #991b1b; background: #fee2e2; border-color: #ef4444; }
.cortex-feedback, .cortex-answer-panel { margin-top: 18px; padding: 14px 16px; border-radius: 10px; background: #f8fafc; border: 1px solid #cbd5e1; }
.cortex-feedback.cortex-feedback-correct { color: #166534; background: #f0fdf4; border-color: #86efac; }
.cortex-feedback.cortex-feedback-wrong { color: #991b1b; background: #fef2f2; border-color: #fca5a5; }
.cortex-explanation { margin-top: 10px; color: #475569; }
.nightMode .card { color: #e5e7eb; background: #1f2937; }
.nightMode .cortex-choice { color: #e5e7eb; background: #273449; border-color: #475569; }
.nightMode .cortex-choice:hover:not(:disabled) { background: #303d56; border-color: #818cf8; }
.nightMode .cortex-choice.cortex-correct { color: #bbf7d0; background: #14532d; border-color: #4ade80; }
.nightMode .cortex-choice.cortex-wrong { color: #fecaca; background: #7f1d1d; border-color: #f87171; }
.nightMode .cortex-feedback, .nightMode .cortex-answer-panel { color: #e5e7eb; background: #273449; border-color: #475569; }
.nightMode .cortex-feedback.cortex-feedback-correct { color: #bbf7d0; background: #14532d; border-color: #4ade80; }
.nightMode .cortex-feedback.cortex-feedback-wrong { color: #fecaca; background: #7f1d1d; border-color: #f87171; }
.nightMode .cortex-explanation { color: #cbd5e1; }
"#,
    qfmt: r#"
{{Front}}
<script>
(function () {
  var root = document.querySelector('.cortex-mcq[data-cortex-mcq]');
  if (!root || root.dataset.cortexBound === '1') return;
  root.dataset.cortexBound = '1';
  var correctIndex = Number(root.dataset.answerIndex);
  var choices = Array.prototype.slice.call(root.querySelectorAll('.cortex-choice'));
  var feedback = root.querySelector('.cortex-feedback');
  var answerData = root.querySelector('.cortex-answer-data');
  choices.forEach(function (choice) {
    choice.addEventListener('click', function () {
      if (root.dataset.cortexAnswered === '1') return;
      root.dataset.cortexAnswered = '1';
      var picked = Number(choice.dataset.index);
      choices.forEach(function (button) {
        button.disabled = true;
        if (Number(button.dataset.index) === correctIndex) button.classList.add('cortex-correct');
      });
      var isCorrect = picked === correctIndex;
      if (!isCorrect) choice.classList.add('cortex-wrong');
      if (!feedback || !answerData) return;
      feedback.classList.add(isCorrect ? 'cortex-feedback-correct' : 'cortex-feedback-wrong');
      feedback.innerHTML = (isCorrect ? '<strong>✓ 回答正确</strong>' : '<strong>✕ 回答错误</strong>') + answerData.innerHTML;
      // Match Anki's own “Show Answer” button after a short confirmation beat,
      // so its native Again/Hard/Good/Easy controls appear without a second tap.
      window.setTimeout(function () {
        if (typeof pycmd === 'function') pycmd('ans');
      }, 550);
    });
  });
})();
</script>
"#,
    afmt: r#"
{{FrontSide}}
<hr id=answer>
<div class="cortex-answer-panel">{{Back}}</div>
<script>
(function () {
  var root = document.querySelector('.cortex-mcq[data-cortex-mcq]');
  if (!root) return;
  var correctIndex = Number(root.dataset.answerIndex);
  Array.prototype.slice.call(root.querySelectorAll('.cortex-choice')).forEach(function (choice) {
    choice.disabled = true;
    if (Number(choice.dataset.index) === correctIndex) choice.classList.add('cortex-correct');
  });
})();
</script>
"#,
};

/// Build a `collection.anki2` SQLite file at `path` containing one Basic note +
/// card per (front, back) pair, all in a deck named `deck_name`, with a separate
/// stable identity seed. Direct Cortex → Anki hand-offs use a material id here,
/// so renaming a material does not cause it to appear as different Anki notes.
fn build_collection_with_identity(
    path: &Path,
    deck_name: &str,
    cards: &[(String, String)],
    identity: &str,
    template: AnkiTemplate,
) -> Result<()> {
    // Start from a clean file — a stale temp left by a crashed prior run would
    // already hold the `col` table and make CREATE TABLE fail ("already exists").
    let _ = std::fs::remove_file(path);
    let conn = Connection::open(path)?;
    conn.execute_batch(COL_SCHEMA)?;

    let now = now_ms();
    let crt = now / 1000;
    let mid = now; // model id
    let did = now + 1; // deck id

    let model = serde_json::json!({
        mid.to_string(): {
            "id": mid, "name": template.name, "type": 0, "mod": crt, "usn": -1,
            "sortf": 0, "did": did, "latexPre": "", "latexPost": "", "latexsvg": false,
            "css": template.css,
            "flds": [
                {"name":"Front","ord":0,"sticky":false,"rtl":false,"font":"Arial","size":20,"media":[]},
                {"name":"Back","ord":1,"sticky":false,"rtl":false,"font":"Arial","size":20,"media":[]}
            ],
            "tmpls": [
                {"name":"Card 1","ord":0,"qfmt":template.qfmt,
                 "afmt":template.afmt,"did":null,"bqfmt":"","bafmt":""}
            ],
            "req": [[0, "any", [0]]], "tags": [], "vers": []
        }
    });
    let decks = serde_json::json!({
        "1": {"id":1,"name":"Default","mod":crt,"usn":-1,"lrnToday":[0,0],"revToday":[0,0],
              "newToday":[0,0],"timeToday":[0,0],"collapsed":false,"browserCollapsed":false,
              "desc":"","dyn":0,"conf":1,"extendNew":0,"extendRev":0},
        did.to_string(): {"id":did,"name":deck_name,"mod":crt,"usn":-1,"lrnToday":[0,0],
              "revToday":[0,0],"newToday":[0,0],"timeToday":[0,0],"collapsed":false,
              "browserCollapsed":false,"desc":"","dyn":0,"conf":1,"extendNew":0,"extendRev":0}
    });
    let dconf = serde_json::json!({
        "1": {"id":1,"name":"Default","mod":0,"usn":0,"maxTaken":60,"autoplay":true,
              "timer":0,"replayq":true,"new":{"bury":false,"delays":[1.0,10.0],"initialFactor":2500,
              "ints":[1,4,0],"order":1,"perDay":20},"rev":{"bury":false,"ease4":1.3,"ivlFct":1.0,
              "maxIvl":36500,"perDay":200,"hardFactor":1.2},"lapse":{"delays":[10.0],"leechAction":1,
              "leechFails":8,"minInt":1,"mult":0.0},"dyn":false}
    });
    let conf = serde_json::json!({
        "nextPos":1,"estTimes":true,"activeDecks":[1],"sortType":"noteFld","timeLim":0,
        "sortBackwards":false,"addToCur":true,"curDeck":did,"newBury":true,"newSpread":0,
        "dueCounts":true,"curModel":mid.to_string(),"collapseTime":1200
    });

    conn.execute(
        "INSERT INTO col (id,crt,mod,scm,ver,dty,usn,ls,conf,models,decks,dconf,tags)
         VALUES (1,?1,?2,?2,11,0,0,0,?3,?4,?5,?6,'{}')",
        params![crt, now, conf.to_string(), model.to_string(), decks.to_string(), dconf.to_string()],
    )?;

    for (idx, (front, back)) in cards.iter().enumerate() {
        let nid = now + 100 + idx as i64;
        let cid = now + 100_000 + idx as i64;
        let flds = format!("{front}\u{001f}{back}");
        let guid = note_guid(identity, front);
        conn.execute(
            "INSERT INTO notes (id,guid,mid,mod,usn,tags,flds,sfld,csum,flags,data)
             VALUES (?1,?2,?3,?4,-1,'',?5,?6,?7,0,'')",
            params![nid, guid, mid, crt, flds, front, field_checksum(front)],
        )?;
        // New card: type/queue 0, due = position (1-based), default ease 2500.
        conn.execute(
            "INSERT INTO cards (id,nid,did,ord,mod,usn,type,queue,due,ivl,factor,reps,lapses,left,odue,odid,flags,data)
             VALUES (?1,?2,?3,0,?4,-1,0,0,?5,0,2500,0,0,0,0,0,0,'')",
            params![cid, nid, did, crt, (idx as i64) + 1],
        )?;
    }
    conn.pragma_update(None, "user_version", 0)?;
    drop(conn);
    Ok(())
}

/// Build an `.apkg` with a stable caller-supplied note identity. This is used
/// when a Cortex material is directly opened in Anki; manual exports retain the
/// deck-name identity used by [`export_apkg`].
pub fn export_apkg_with_identity(
    dest: &Path,
    deck_name: &str,
    cards: &[(String, String)],
    identity: &str,
) -> Result<()> {
    export_apkg_with_template(dest, deck_name, cards, identity, BASIC_TEMPLATE)
}

/// Same package writer for the interactive quiz note type.
pub fn export_quiz_apkg_with_identity(
    dest: &Path,
    deck_name: &str,
    cards: &[(String, String)],
    identity: &str,
) -> Result<()> {
    export_apkg_with_template(dest, deck_name, cards, identity, MULTIPLE_CHOICE_TEMPLATE)
}

fn export_apkg_with_template(
    dest: &Path,
    deck_name: &str,
    cards: &[(String, String)],
    identity: &str,
    template: AnkiTemplate,
) -> Result<()> {
    if cards.is_empty() {
        return Err(Error::Other("no flashcards to export".into()));
    }
    // Build collection.anki2 in a temp file, then read its bytes.
    let tmp = std::env::temp_dir().join(format!("cortex-anki-{}.anki2", temp_token()));
    build_collection_with_identity(&tmp, deck_name, cards, identity, template)?;
    let col_bytes = std::fs::read(&tmp).map_err(Error::Io)?;
    let _ = std::fs::remove_file(&tmp);

    let zip = zip_store(&[
        ("collection.anki2", col_bytes),
        ("media", b"{}".to_vec()),
    ]);
    std::fs::write(dest, zip).map_err(Error::Io)?;
    Ok(())
}

/// Escape a Cortex text field for Anki's HTML renderer while preserving line
/// breaks. We intentionally leave Markdown syntax as plain text: existing Cortex
/// cards use it, and Anki's Basic template does not understand Markdown.
fn anki_html(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\n' => out.push_str("<br>"),
            _ => out.push(ch),
        }
    }
    out
}

fn option_letter(index: usize) -> char {
    (b'A' + (index % 26) as u8) as char
}

fn quiz_answer_html(
    answer_index: usize,
    answer: &str,
    explain: Option<&str>,
    tags: &[String],
) -> String {
    let mut out = format!(
        "<br><br><strong>正确答案：</strong>{}. {}",
        option_letter(answer_index),
        anki_html(answer)
    );
    if let Some(explain) = explain.filter(|value| !value.is_empty()) {
        out.push_str(&format!("<div class=\"cortex-explanation\"><strong>解析：</strong>{}</div>", anki_html(explain)));
    }
    if !tags.is_empty() {
        out.push_str(&format!("<div class=\"cortex-explanation\"><small>标签：{}</small></div>", tags.iter().map(|tag| anki_html(tag)).collect::<Vec<_>>().join(" · ")));
    }
    out
}

fn interactive_quiz_front(
    question: &str,
    options: &[String],
    answer_index: usize,
    answer_html: &str,
) -> String {
    let choices = options
        .iter()
        .enumerate()
        .map(|(index, option)| format!(
            "<button class=\"cortex-choice\" type=\"button\" data-index=\"{index}\"><span class=\"cortex-choice-key\">{}</span><span>{}</span></button>",
            option_letter(index),
            anki_html(option)
        ))
        .collect::<String>();
    format!(
        "<div class=\"cortex-mcq\" data-cortex-mcq data-answer-index=\"{answer_index}\"><div class=\"cortex-question\">{}</div><div class=\"cortex-options\">{choices}</div><div class=\"cortex-answer-data\" hidden>{answer_html}</div><div class=\"cortex-feedback\" aria-live=\"polite\"></div></div>",
        anki_html(question)
    )
}

/// Turn one Cortex material payload into Anki front/back pairs. Flashcards
/// retain their normal Basic model; quiz cards use a dedicated Anki template
/// that turns their A/B/C/D choices into clickable controls.
pub fn cards_from_material(
    kind: &str,
    payload: &serde_json::Value,
) -> Result<Vec<(String, String)>> {
    let items = payload
        .as_array()
        .ok_or_else(|| Error::Other("material payload is not a card list".into()))?;
    let cards: Vec<(String, String)> = match kind {
        "flashcards" => items
            .iter()
            .filter_map(|item| {
                let front = item["q"].as_str()?.trim();
                let back = item["a"].as_str().unwrap_or("").trim();
                (!front.is_empty()).then(|| (anki_html(front), anki_html(back)))
            })
            .collect(),
        "quiz" => items
            .iter()
            .filter_map(|item| {
                let question = item["q"].as_str()?.trim();
                let options = item["options"].as_array()?;
                let answer_index = item["answer"].as_u64()? as usize;
                let answer = options.get(answer_index)?.as_str()?.trim();
                if question.is_empty() || answer.is_empty() {
                    return None;
                }
                let options: Vec<String> = options
                    .iter()
                    .map(|option| option.as_str().unwrap_or("").trim().to_string())
                    .collect();
                if options.iter().any(|option| option.is_empty()) {
                    return None;
                }
                let explain = item["explain"].as_str().map(str::trim).filter(|v| !v.is_empty());
                let tags: Vec<String> = item["tags"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|tag| tag.as_str().map(str::trim).filter(|tag| !tag.is_empty()))
                    .map(str::to_string)
                    .collect();
                let back = quiz_answer_html(answer_index, answer, explain, &tags);
                let front = interactive_quiz_front(question, &options, answer_index, &back);
                Some((front, back))
            })
            .collect(),
        other => {
            return Err(Error::Other(format!(
                "Anki import supports flashcards and quizzes (this is a {other} material)."
            )))
        }
    };
    if cards.is_empty() {
        return Err(Error::Other("this material has no cards to send to Anki".into()));
    }
    Ok(cards)
}

// ===== IMPORT: read an `.apkg` into (deck name → cards) ======================
//
// An `.apkg` is a ZIP holding a `collection.anki21` (newer) or `collection.anki2`
// (older) member — both are plain SQLite databases in Anki's schema. We extract
// that member to a temp file, open it read-only with rusqlite, then pull:
//   • the deck id→name map from the single `col` row's `decks` JSON column, and
//   • every note's fields (`notes.flds`, fields joined by the 0x1f unit
//     separator), with each note's deck resolved via its first card's `did`.
// Fronts/backs are HTML-stripped + entity-decoded so they match how Cortex stores
// generated cards (plain text the Flashcards view renders with RichText). The hard
// `MAX_IMPORT` cap keeps a pathological deck from ballooning the DB / UI.

/// Hard ceiling on cards per import — beyond this we error rather than ingest an
/// unbounded deck (protects the materials table and the in-memory deck render).
pub const MAX_IMPORT: usize = 5000;

/// A single imported card: HTML-stripped, trimmed front/back.
#[derive(Debug, Clone)]
pub struct ImportedCard {
    pub front: String,
    pub back: String,
}

/// One imported deck: a name plus its cards (already deduped within the deck).
#[derive(Debug, Clone)]
pub struct ImportedDeck {
    pub name: String,
    pub cards: Vec<ImportedCard>,
}

/// Decode the handful of HTML entities that show up in Anki fields. We only need
/// the common five (plus numeric refs) — fields are mostly plain text, and full
/// entity tables would be overkill. Mirrors the codebase's "minimal, explain why"
/// style: anything exotic is left as-is rather than risking a wrong substitution.
fn decode_entities(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(amp) = rest.find('&') {
        out.push_str(&rest[..amp]);
        rest = &rest[amp..];
        // Find the terminating ';' within a short window (real entities are short).
        if let Some(semi) = rest[..rest.len().min(12)].find(';') {
            let ent = &rest[1..semi];
            let decoded = match ent {
                "amp" => Some('&'),
                "lt" => Some('<'),
                "gt" => Some('>'),
                "quot" => Some('"'),
                "apos" => Some('\''),
                "nbsp" => Some(' '),
                _ => ent
                    .strip_prefix('#')
                    .and_then(|num| {
                        if let Some(hex) = num.strip_prefix(['x', 'X']) {
                            u32::from_str_radix(hex, 16).ok()
                        } else {
                            num.parse::<u32>().ok()
                        }
                    })
                    .and_then(char::from_u32),
            };
            match decoded {
                Some(ch) => {
                    out.push(ch);
                    rest = &rest[semi + 1..];
                }
                // Unknown entity — keep the '&' literally and move past it so we
                // don't loop forever on the same position.
                None => {
                    out.push('&');
                    rest = &rest[1..];
                }
            }
        } else {
            out.push('&');
            rest = &rest[1..];
        }
    }
    out.push_str(rest);
    out
}

/// Normalize a field for storage/dedupe: strip HTML, decode entities, collapse
/// runs of whitespace (Anki fields carry `<br>` and stray newlines), and trim.
fn clean_field(s: &str) -> String {
    let text = decode_entities(&strip_html(s));
    // Collapse all whitespace (incl. the newlines left by stripped <br>/<div>) to
    // single spaces so dedupe is robust and the card renders on one logical line.
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Case-insensitive key for duplicate detection. Inputs are already cleaned by
/// `clean_field` (HTML stripped, whitespace collapsed), so two fronts differing
/// only by letter-case map to the same key here.
pub fn dedupe_key(front: &str) -> String {
    front.to_lowercase()
}

/// Extract the named collection member from an `.apkg` zip to a temp file and
/// return its path. Prefers `collection.anki21`, falling back to `collection.anki2`.
fn extract_collection(apkg: &Path) -> Result<std::path::PathBuf> {
    let file = std::fs::File::open(apkg)
        .map_err(|e| Error::Other(format!("could not open .apkg file: {e}")))?;
    let mut archive = zip::ZipArchive::new(file)
        .map_err(|e| Error::Other(format!("not a valid .apkg (zip) file: {e}")))?;

    // Newer Anki names the DB collection.anki21; older uses collection.anki2.
    let member = ["collection.anki21", "collection.anki2"]
        .into_iter()
        .find(|name| archive.by_name(name).is_ok())
        .ok_or_else(|| {
            Error::Other(
                "not a valid .apkg: no collection.anki21 or collection.anki2 inside".into(),
            )
        })?;

    let mut entry = archive
        .by_name(member)
        .map_err(|e| Error::Other(format!("could not read {member} from .apkg: {e}")))?;
    let dest = std::env::temp_dir().join(format!("cortex-anki-import-{}.db", temp_token()));
    let mut out = std::fs::File::create(&dest).map_err(Error::Io)?;
    std::io::copy(&mut entry, &mut out).map_err(Error::Io)?;
    drop(out);
    Ok(dest)
}

/// Read an `.apkg` at `path` into a list of decks with cleaned cards. Decks with
/// no usable cards are dropped. Errors clearly on a corrupt zip, a missing
/// collection member, or an unreadable database. Cleans up its temp file always.
pub fn import_apkg(path: &Path) -> Result<Vec<ImportedDeck>> {
    let db_path = extract_collection(path)?;
    // RAII-style cleanup: ensure the temp DB is removed on every exit path below.
    let result = read_collection(&db_path);
    let _ = std::fs::remove_file(&db_path);
    result
}

/// Open the extracted collection DB and assemble decks. Split out from
/// `import_apkg` so the temp-file cleanup wraps it unconditionally.
fn read_collection(db_path: &Path) -> Result<Vec<ImportedDeck>> {
    // Plain connection — this is a foreign DB; we register no extensions (no
    // sqlite-vec) and only read from it.
    let conn = Connection::open(db_path)
        .map_err(|e| Error::Other(format!("not a valid .apkg: unreadable collection db ({e})")))?;

    // Deck id → name, from the single `col` row's `decks` JSON object.
    let decks_json: String = conn
        .query_row("SELECT decks FROM col LIMIT 1", [], |r| r.get(0))
        .map_err(|e| Error::Other(format!("not a valid .apkg: missing col table ({e})")))?;
    let decks_val: serde_json::Value = serde_json::from_str(&decks_json)
        .map_err(|e| Error::Other(format!("malformed decks data in .apkg ({e})")))?;
    let mut deck_names: std::collections::HashMap<i64, String> = std::collections::HashMap::new();
    if let Some(obj) = decks_val.as_object() {
        for (id, d) in obj {
            if let (Ok(did), Some(name)) = (id.parse::<i64>(), d.get("name").and_then(|n| n.as_str()))
            {
                deck_names.insert(did, name.to_string());
            }
        }
    }

    // Join notes → their (first) card to learn which deck each note belongs to.
    // A note can in theory have several cards across decks; for Basic decks (the
    // import target) it's one, so MIN(did) is a safe, deterministic choice.
    let mut stmt = conn
        .prepare(
            "SELECT n.flds, COALESCE(MIN(c.did), 0) AS did \
             FROM notes n LEFT JOIN cards c ON c.nid = n.id \
             GROUP BY n.id",
        )
        .map_err(|e| Error::Other(format!("not a valid .apkg: missing notes/cards ({e})")))?;

    // Preserve deck encounter order so the resulting materials feel stable.
    let mut order: Vec<i64> = Vec::new();
    let mut by_deck: std::collections::HashMap<i64, Vec<ImportedCard>> =
        std::collections::HashMap::new();
    // Per-deck seen-fronts for within-deck dedupe.
    let mut seen: std::collections::HashMap<i64, std::collections::HashSet<String>> =
        std::collections::HashMap::new();

    let rows = stmt
        .query_map([], |r| {
            let flds: String = r.get(0)?;
            let did: i64 = r.get(1)?;
            Ok((flds, did))
        })
        .map_err(Error::Db)?;

    let mut total = 0usize;
    for row in rows {
        let (flds, did) = row.map_err(Error::Db)?;
        // Fields are joined by the 0x1f unit separator. First = front, second = back.
        let mut parts = flds.split('\u{001f}');
        let front = clean_field(parts.next().unwrap_or(""));
        let back = clean_field(parts.next().unwrap_or(""));
        if front.is_empty() {
            continue; // unusable card — skip (counted as skipped by the caller)
        }
        let set = seen.entry(did).or_default();
        if !set.insert(dedupe_key(&front)) {
            continue; // duplicate front within this deck
        }
        if !by_deck.contains_key(&did) {
            order.push(did);
        }
        by_deck.entry(did).or_default().push(ImportedCard { front, back });
        total += 1;
        if total > MAX_IMPORT {
            return Err(Error::Other(format!(
                "this .apkg has more than {MAX_IMPORT} cards — split it and import in parts"
            )));
        }
    }

    let decks = order
        .into_iter()
        .filter_map(|did| {
            let cards = by_deck.remove(&did)?;
            if cards.is_empty() {
                return None;
            }
            let name = deck_names
                .get(&did)
                .cloned()
                .unwrap_or_else(|| "Imported deck".to_string());
            Some(ImportedDeck { name, cards })
        })
        .collect();
    Ok(decks)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha1_known_vector() {
        let d = sha1(b"abc");
        let hex: String = d.iter().map(|b| format!("{b:02x}")).collect();
        assert_eq!(hex, "a9993e364706816aba3e25717850c26c9cd0d89d");
    }

    #[test]
    fn crc32_known_vector() {
        // CRC-32 of "123456789" is 0xCBF43926.
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
    }

    #[test]
    fn apkg_is_a_valid_zip_with_anki_collection() {
        let dir = std::env::temp_dir().join(format!("cortex-apkg-test-{}", now_ms()));
        std::fs::create_dir_all(&dir).unwrap();
        let dest = dir.join("deck.apkg");
        let cards = vec![
            ("What is ATP?".to_string(), "Adenosine triphosphate".to_string()),
            ("Powerhouse of the cell?".to_string(), "Mitochondria".to_string()),
        ];
        export_apkg_with_identity(&dest, "Biology", &cards, "Biology").unwrap();

        let bytes = std::fs::read(&dest).unwrap();
        // ZIP local-file signature at the start, EOCD signature near the end.
        assert_eq!(&bytes[0..4], &[0x50, 0x4b, 0x03, 0x04]);
        assert!(bytes.windows(4).any(|w| w == [0x50, 0x4b, 0x05, 0x06]));
        // Contains both archive members.
        assert!(bytes.windows(b"collection.anki2".len()).any(|w| w == b"collection.anki2"));
        assert!(bytes.windows(b"media".len()).any(|w| w == b"media"));

        // The embedded collection.anki2 must be a real SQLite DB with 2 notes/cards.
        let tmp = dir.join("roundtrip.anki2");
        build_collection_with_identity(&tmp, "Biology", &cards, "Biology", BASIC_TEMPLATE).unwrap();
        let conn = Connection::open(&tmp).unwrap();
        let notes: i64 = conn.query_row("SELECT count(*) FROM notes", [], |r| r.get(0)).unwrap();
        let cnt: i64 = conn.query_row("SELECT count(*) FROM cards", [], |r| r.get(0)).unwrap();
        assert_eq!(notes, 2);
        assert_eq!(cnt, 2);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn quiz_material_becomes_answerable_anki_cards() {
        let payload = serde_json::json!([
            {
                "q": "Where should you look for new terms first?",
                "options": ["Domain", "Blog comments", "Ads library", "Newsletter"],
                "answer": 1,
                "explain": "Comments show real user language.",
                "tags": ["new terms", "research"]
            }
        ]);
        let cards = cards_from_material("quiz", &payload).unwrap();
        assert_eq!(cards.len(), 1);
        assert!(cards[0].0.contains("cortex-mcq"));
        assert!(cards[0].0.contains("cortex-choice"));
        assert!(cards[0].0.contains("Domain"));
        assert!(cards[0].0.contains("Blog comments"));
        assert!(cards[0].1.contains("正确答案：</strong>B. Blog comments"));
        assert!(cards[0].1.contains("解析：</strong>Comments show real user language."));
        assert!(cards[0].1.contains("标签：new terms · research"));

        let dir = std::env::temp_dir().join(format!("cortex-mcq-apkg-{}", now_ms()));
        std::fs::create_dir_all(&dir).unwrap();
        let dest = dir.join("quiz.apkg");
        export_quiz_apkg_with_identity(&dest, "Cortex::Research", &cards, "material-1").unwrap();
        let db = extract_collection(&dest).unwrap();
        let conn = Connection::open(&db).unwrap();
        let models: String = conn.query_row("SELECT models FROM col", [], |row| row.get(0)).unwrap();
        assert!(models.contains("Cortex Multiple Choice"));
        assert!(models.contains("cortex-choice"));
        assert!(models.contains("pycmd('ans')"));
        let _ = std::fs::remove_file(&db);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn note_guid_is_stable_per_deck_and_front() {
        assert_eq!(
            note_guid("Cortex::Research", "Find new terms"),
            note_guid("Cortex::Research", "Find new terms")
        );
        assert_ne!(
            note_guid("Cortex::Research", "Find new terms"),
            note_guid("Cortex::Research", "Validate terms")
        );
    }

    #[test]
    fn entities_and_html_are_cleaned() {
        assert_eq!(decode_entities("a &amp; b &lt;c&gt; &quot;d&quot;"), "a & b <c> \"d\"");
        assert_eq!(decode_entities("x&nbsp;y"), "x y");
        assert_eq!(decode_entities("&#65;&#x42;"), "AB");
        // Unknown entity is left intact (no infinite loop, no wrong substitution).
        assert_eq!(decode_entities("100% &foo; ok"), "100% &foo; ok");
        // Full field cleaning: strip tags, decode known entities, collapse
        // whitespace. Unknown entities (&rarr;) are preserved verbatim by design.
        assert_eq!(clean_field("<b>Na</b>+ &amp;<br>  ion"), "Na+ & ion");
        assert_eq!(clean_field("  <div>What is ATP?</div> "), "What is ATP?");
    }

    #[test]
    fn import_roundtrips_an_exported_apkg() {
        let dir = std::env::temp_dir().join(format!("cortex-apkg-import-{}", now_ms()));
        std::fs::create_dir_all(&dir).unwrap();
        let dest = dir.join("deck.apkg");
        let cards = vec![
            ("What is ATP?".to_string(), "Adenosine triphosphate".to_string()),
            ("Powerhouse of the cell?".to_string(), "Mitochondria".to_string()),
        ];
        export_apkg_with_identity(&dest, "Biology", &cards, "Biology").unwrap();

        let decks = import_apkg(&dest).unwrap();
        assert_eq!(decks.len(), 1, "one Anki deck → one imported deck");
        let deck = &decks[0];
        assert_eq!(deck.name, "Biology");
        assert_eq!(deck.cards.len(), 2);
        assert_eq!(deck.cards[0].front, "What is ATP?");
        assert_eq!(deck.cards[0].back, "Adenosine triphosphate");
        assert_eq!(deck.cards[1].front, "Powerhouse of the cell?");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn import_dedupes_within_deck_and_skips_empty_fronts() {
        let dir = std::env::temp_dir().join(format!("cortex-apkg-dedupe-{}", now_ms()));
        std::fs::create_dir_all(&dir).unwrap();
        let dest = dir.join("deck.apkg");
        // Two identical fronts (differing only by case) + one empty front.
        let cards = vec![
            ("Term".to_string(), "Def".to_string()),
            ("term".to_string(), "Other".to_string()), // dup of "Term" (case-insensitive)
            ("".to_string(), "no front".to_string()),  // empty front → skipped
            ("Unique".to_string(), "Yes".to_string()),
        ];
        export_apkg_with_identity(&dest, "Vocab", &cards, "Vocab").unwrap();

        let decks = import_apkg(&dest).unwrap();
        assert_eq!(decks.len(), 1);
        // "Term", (dup dropped), (empty dropped), "Unique" → 2 cards.
        assert_eq!(decks[0].cards.len(), 2);
        let fronts: Vec<&str> = decks[0].cards.iter().map(|c| c.front.as_str()).collect();
        assert!(fronts.contains(&"Term"));
        assert!(fronts.contains(&"Unique"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn import_rejects_a_non_apkg_file() {
        let dir = std::env::temp_dir().join(format!("cortex-apkg-bad-{}", now_ms()));
        std::fs::create_dir_all(&dir).unwrap();
        let bad = dir.join("not.apkg");
        std::fs::write(&bad, b"this is not a zip file at all").unwrap();
        let err = import_apkg(&bad).unwrap_err();
        assert!(err.to_string().contains("not a valid .apkg"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}

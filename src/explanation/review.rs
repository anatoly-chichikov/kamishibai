//! Pure, language-data-driven explanation acceptance and repair diagnostics.

use std::collections::BTreeSet;

use fancy_regex::Regex;

use super::{Card, Language, Parts, compose, quote};
use crate::markdown::{Block, TextChunk, parse_markdown};

/// A composed explanation and every actionable reason it cannot replace the draft.
#[derive(Clone, Debug)]
pub struct Verdict {
    fails: Vec<String>,
    markdown: String,
}

impl Verdict {
    /// Consume the review into repair diagnostics and the exact composed text.
    #[must_use]
    pub fn into_parts(self) -> (Vec<String>, String) {
        (self.fails, self.markdown)
    }
}

fn regex(pattern: &str) -> Regex {
    Regex::new(pattern).expect("invariant: explanation rule pattern must compile")
}

fn matches(pattern: &str, text: &str) -> bool {
    regex(pattern)
        .is_match(text)
        .expect("invariant: bounded explanation rule must execute")
}

fn hits(pattern: &str, text: &str) -> Vec<String> {
    regex(pattern)
        .find_iter(text)
        .map(|hit| {
            hit.expect("invariant: bounded explanation pattern must execute")
                .as_str()
                .to_string()
        })
        .collect()
}

fn size(text: &str) -> usize {
    text.encode_utf16().count()
}
fn letters(text: &str) -> usize {
    text.chars().filter(|ch| ch.is_alphabetic()).count()
}
fn stems(text: &str) -> BTreeSet<String> {
    hits(r"[\p{L}]{4,}", &text.to_lowercase().replace('ё', "е"))
        .into_iter()
        .map(|word| {
            word.chars()
                .take(5.min(3.max(word.chars().count() - 1)))
                .collect()
        })
        .collect()
}
fn bare(text: &str) -> String {
    regex(r"[^\p{L}0-9]+")
        .replace_all(&text.to_lowercase().replace('ё', "е"), " ")
        .trim()
        .to_string()
}
fn sentences(text: &str) -> Vec<&str> {
    let mut result = Vec::new();
    let mut start = 0;
    for found in regex(r"(?<=[.!?])\s+").find_iter(text) {
        let found = found.expect("invariant: sentence split must execute");
        let piece = text[start..found.start()].trim();
        if !piece.is_empty() {
            result.push(piece);
        }
        start = found.end();
    }
    let piece = text[start..].trim();
    if !piece.is_empty() {
        result.push(piece);
    }
    result
}
fn unique(items: Vec<String>) -> Vec<String> {
    let mut seen = BTreeSet::new();
    items
        .into_iter()
        .filter(|item| seen.insert(item.clone()))
        .collect()
}
fn chunks(block: &Block) -> &[TextChunk] {
    match block {
        Block::Paragraph(chunks) | Block::Bullet { chunks, .. } => chunks,
    }
}
fn text(block: &Block) -> String {
    chunks(block)
        .iter()
        .map(|chunk| chunk.text.as_str())
        .collect()
}
fn shared(left: &str, right: &str, term: &str) -> bool {
    let mut skip = stems(
        "говорят говорит скажут скажешь сказать звучит слово слова часто обычно чаще просто когда если также тоже именно здесь",
    );
    skip.extend(stems(term));
    let one = stems(left)
        .difference(&skip)
        .cloned()
        .collect::<BTreeSet<_>>();
    let two = stems(right)
        .difference(&skip)
        .cloned()
        .collect::<BTreeSet<_>>();
    one.len() >= 4
        && two.len() >= 4
        && one.intersection(&two).count() * 10 >= one.len().min(two.len()) * 6
}

fn paired(text: &str, open: &str, close: &str) -> bool {
    let open = open.trim();
    let close = close.trim();
    let mut inside = false;
    let mut remaining = text;
    while !remaining.is_empty() {
        if let Some(rest) = remaining.strip_prefix(if inside { close } else { open }) {
            inside = !inside;
            remaining = rest;
        } else if remaining.starts_with(open) || remaining.starts_with(close) {
            return false;
        } else {
            remaining = &remaining[remaining
                .chars()
                .next()
                .expect("invariant: remaining text is nonempty")
                .len_utf8()..];
        }
    }
    !inside
}

struct Review<'a> {
    language: &'a Language,
    fails: Vec<String>,
}

impl Review<'_> {
    fn fail(&mut self, rule: &str, reason: impl AsRef<str>) {
        self.fails.push(format!("{rule}: {}", reason.as_ref()));
    }

    fn local(&self, name: &str, text: &str) -> bool {
        self.language
            .context
            .judge
            .get(name)
            .is_some_and(|pattern| matches(pattern, &text.replace(['⟦', '⟧'], "")))
    }

    fn local_hits(&self, name: &str, text: &str) -> Vec<String> {
        self.language
            .context
            .judge
            .get(name)
            .map(|pattern| hits(pattern, &text.replace(['⟦', '⟧'], "")))
            .unwrap_or_default()
    }

    fn mistake(&self, text: &str) -> bool {
        let marked = regex("⟦[^⟦⟧]+⟧").replace_all(text, "a");
        let foreign = text
            .chars()
            .map(|ch| {
                if ch.is_alphabetic() && self.language.foreign(ch) {
                    'a'
                } else {
                    ch
                }
            })
            .collect::<String>();
        self.local("mistake", text)
            || self.local("mistake", &marked)
            || self.local("mistake", &foreign)
    }

    fn learning_words(&self, text: &str) -> Vec<String> {
        use unicode_script::UnicodeScript;
        let marked = hits("⟦[^⟦⟧]+⟧", text);
        let phrases = if marked.is_empty() {
            text.to_string()
        } else {
            marked.join(" ")
        };
        hits(r"\p{L}[\p{L}\p{M}'’-]*", &phrases.to_lowercase())
            .into_iter()
            .filter(|word| {
                word.chars()
                    .any(|ch| self.language.learning.contains(&ch.script()))
            })
            .collect()
    }

    fn list(&mut self, card: &Card, parts: &Parts) {
        if card.senses.is_empty() {
            for line in &parts.meanings {
                if !line.from.trim().is_empty() && !card.translation.contains(line.from.trim()) {
                    self.fail("list", format!("\"from\" of the line «{}» is «{}», which is not a piece of the translation copied character for character", line.head, line.from));
                }
            }
            for group in card
                .translation
                .split(';')
                .map(str::trim)
                .filter(|group| !group.is_empty())
            {
                if !parts
                    .meanings
                    .iter()
                    .any(|line| !line.from.trim().is_empty() && group.contains(line.from.trim()))
                {
                    self.fail("list", format!("the meaning «{group}» of the translation has no line of its own in \"meanings\""));
                }
            }
        } else if parts.meanings.len() != card.senses.len()
            || parts
                .meanings
                .iter()
                .zip(&card.senses)
                .any(|(line, sense)| line.from != sense.number)
        {
            self.fail("list", "write exactly one line per reviewed sense, in the same order, with its number in \"from\"");
        }
        let mut seen = BTreeSet::new();
        for line in &parts.meanings {
            if !seen.insert(bare(&line.head)) {
                self.fail(
                    "list",
                    format!("two lines have the same head «{}»", line.head),
                );
            }
        }
        let Some(main) = parts.meanings.first() else {
            return;
        };
        let known = stems(&format!("{} {}", card.translation, main.head));
        let echo = if self.language.names.worded {
            stems(&main.note).difference(&known).count() < 2
        } else {
            bare(&main.note) == bare(&main.head) || bare(&main.note) == bare(&card.translation)
        };
        if echo {
            self.fail("echo", format!("the note of the first line («{}») says the translation again; say what things or what situation it is about", main.note));
        }
        let borrowed = parts
            .meanings
            .iter()
            .skip(1)
            .filter(|line| {
                !stems(&line.head).is_empty() && stems(&line.head).is_subset(&stems(&main.note))
            })
            .map(|line| line.head.as_str())
            .collect::<Vec<_>>();
        if !borrowed.is_empty() {
            self.fail("list", format!("the note of the first line uses «{}», which is the head of another line; say it with other words", borrowed.join("», «")));
        }
    }

    fn structure(&mut self, parts: &Parts, blocks: &[Block]) {
        let heads = &self.language.context.headers;
        let found = blocks
            .iter()
            .filter_map(|block| match block {
                Block::Paragraph(chunks) if chunks.first().is_some_and(|chunk| chunk.bold) => {
                    Some(chunks[0].text.as_str())
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        if !matches!(blocks.first(), Some(Block::Paragraph(chunks)) if chunks.len() == 1 && chunks[0].text == heads[0])
        {
            self.fail(
                "structure",
                "the text does not open with the heading of the meanings",
            );
        }
        let order = found
            .iter()
            .map(|head| heads.iter().position(|known| known == head))
            .collect::<Vec<_>>();
        if order.iter().any(Option::is_none) || order.windows(2).any(|pair| pair[0] >= pair[1]) {
            self.fail("structure", format!("the headings on the card are «{}», expected the four known ones in their order without repeats", found.join("», «")));
        }
        if found.len()
            != 1 + parts
                .sections()
                .iter()
                .filter(|(_, text, _)| !text.trim().is_empty())
                .count()
        {
            self.fail(
                "structure",
                "a section lost its heading or a heading stands without its section",
            );
        }
        let count = parts.meanings.len() + usize::from(!parts.literal.trim().is_empty());
        if blocks
            .iter()
            .filter(|block| matches!(block, Block::Bullet { .. }))
            .count()
            != count
            || blocks
                .iter()
                .skip(1)
                .take(count)
                .any(|block| !matches!(block, Block::Bullet { .. }))
            || blocks
                .iter()
                .skip(1 + count)
                .any(|block| matches!(block, Block::Bullet { .. }))
        {
            self.fail("structure", "the list under the first heading differs from \"meanings\"; a string must not start with \"- \" or \"* \"");
        }
        if parts.meanings.first().is_some_and(|main| {
            !blocks.get(1).is_some_and(|block| {
                text(block).starts_with(&format!("{} — ", quote(main.head.trim(), self.language)))
            })
        }) {
            self.fail("highlight", "the first line of the list is not the meaning of the card; the card marks its own meaning by that place");
        }
        if blocks.iter().any(|block| {
            chunks(block).iter().enumerate().any(|(index, chunk)| {
                chunk.italic
                    || (chunk.bold
                        && (!matches!(block, Block::Paragraph(_))
                            || index > 0
                            || !heads.contains(&chunk.text)))
            })
        }) {
            self.fail(
                "markup",
                "bold or italic marks stand outside the four headings; nothing in the text is bold",
            );
        }
    }

    fn quotes(&mut self, parts: &Parts, blocks: &[Block]) {
        let [open, close] = &self.language.context.quotes;
        let body = blocks
            .iter()
            .map(|block| {
                chunks(block)
                    .iter()
                    .filter(|chunk| !chunk.bold)
                    .map(|chunk| chunk.text.as_str())
                    .collect::<String>()
            })
            .collect::<Vec<_>>();
        if body.iter().any(|text| !paired(text, open, close))
            || parts
                .strings()
                .iter()
                .any(|text| !paired(text, "⟦", "⟧") || !paired(text, open, close))
        {
            self.fail(
                "quotes",
                format!("a quotation mark {open} or {close} has no pair"),
            );
        }
        let pattern = format!("{}.*?{}", regex::escape(open), regex::escape(close));
        let bare = body
            .iter()
            .map(|text| regex(&pattern).replace_all(text, " ").to_string())
            .collect::<Vec<_>>()
            .join("\n");
        if bare.chars().any(|ch| self.language.foreign(ch)) {
            self.fail(
                "quotes",
                "words of the learning language stand on the card outside quotation marks",
            );
        }
    }

    fn empty(&mut self, parts: &Parts, whole: &str) {
        for line in &parts.meanings {
            if letters(&line.head) < 1 || letters(&line.note) < 3 {
                self.fail(
                    "empty",
                    format!(
                        "the line «{}» of \"meanings\" lacks a head or a note",
                        line.head
                    ),
                );
            }
        }
        for (name, value, example) in parts.sections() {
            let has = !example.text.trim().is_empty() || !example.translation.trim().is_empty();
            if !value.trim().is_empty() && letters(value) < 12 {
                self.fail("empty", format!("\"{name}\" has too little to be a section; say something real or leave it \"\""));
            }
            if value.trim().is_empty() && has {
                self.fail("empty", format!("\"{name}\" has an example but no text; an empty part cannot have an example"));
            }
            if has && (letters(&example.text) < 2 || letters(&example.translation) < 2) {
                self.fail(
                    "empty",
                    format!("the example of \"{name}\" lacks its sentence or its translation"),
                );
            }
        }
        if self.local("calm", whole) || self.local("blank", whole) {
            self.fail("empty", "a phrase says that there are no restrictions or nothing to add; leave that part \"\" instead");
        }
    }

    fn lengths(&mut self, parts: &Parts, whole: &str) {
        let count = parts.meanings.len() + usize::from(!parts.literal.trim().is_empty());
        if count > 5 {
            self.fail("length", format!("the list has {count} lines with the literal picture, the limit is 5; keep every reviewed sense"));
        }
        for (index, line) in parts.meanings.iter().enumerate() {
            let drawn = format!("{} — {}", line.head.trim(), line.note.trim());
            let limit = if index == 0 { 68 } else { 38 };
            if size(&drawn) > limit {
                self.fail("length", format!("the line «{drawn}» takes {} characters with its head and dash, the limit is {limit}; shorten the note", size(&drawn)));
            }
        }
        if !parts.literal.trim().is_empty()
            && size(&format!(
                "{} — {}",
                self.language.context.literally,
                parts.literal.trim()
            )) > 76
        {
            self.fail(
                "length",
                "\"literal\" exceeds its 76-character line including the label and dash",
            );
        }
        for (name, value, _) in parts.sections() {
            let limit = if name == "nuance" { 150 } else { 170 };
            if size(value.trim()) > limit {
                self.fail(
                    "length",
                    format!(
                        "the text of \"{name}\" takes {} characters, the limit is {limit}",
                        size(value.trim())
                    ),
                );
            }
        }
        let examples = parts
            .sections()
            .into_iter()
            .filter(|(_, _, example)| !example.text.trim().is_empty())
            .collect::<Vec<_>>();
        for (_, _, example) in &examples {
            if size(example.text.trim()) > 65 {
                self.fail(
                    "length",
                    format!(
                        "the example «{}» takes {} characters, the limit is 65",
                        example.text,
                        size(example.text.trim())
                    ),
                );
            }
            if size(example.translation.trim()) + 2 > 85 {
                self.fail(
                    "length",
                    format!(
                        "the translation «{}» is too long, the limit is 83",
                        example.translation
                    ),
                );
            }
        }
        if examples.len() > 2 {
            self.fail(
                "length",
                format!("{} examples, the limit is 2", examples.len()),
            );
        }
        if size(whole) > 700 {
            self.fail(
                "length",
                format!(
                    "the whole text takes {} characters, the limit is 700",
                    size(whole)
                ),
            );
        }
    }

    fn why(&mut self, parts: &Parts) {
        let reason = parts.nuance.why.trim();
        if self.mistake(&parts.nuance.text) {
            if letters(reason) < 8 {
                self.fail("why", "\"nuance\" names a mistake, so \"why\" must say in one short sentence why the wrong phrase does not work: what the right small word shows here or what the wrong one would mean");
            } else {
                if size(reason) > 80 {
                    self.fail(
                        "why",
                        format!(
                            "\"why\" takes {} characters, the limit is 80; say it shorter",
                            size(reason)
                        ),
                    );
                }
                if sentences(reason).len() != 1
                    || !(super::layout::terminated(reason) || reason.ends_with('»'))
                {
                    self.fail("why", "\"why\" must be one full sentence");
                }
                let words = self.learning_words(&parts.nuance.text);
                if !self
                    .learning_words(reason)
                    .iter()
                    .any(|word| words.contains(word))
                {
                    self.fail("why", "\"why\" must speak about the very word that differs in the two phrases of \"nuance\" and name it");
                }
                if self.mistake(reason) {
                    self.fail(
                        "why",
                        "\"why\" repeats the pair of phrases; it must only give the reason",
                    );
                }
            }
        } else if !reason.is_empty() && self.language.context.judge.contains_key("mistake") {
            self.fail(
                "why",
                "\"why\" is filled though \"nuance\" names no mistake; leave it \"\"",
            );
        }
    }

    fn wording(&mut self, card: &Card, parts: &Parts, whole: &str) {
        if matches("[;；]", whole) {
            self.fail(
                "semicolon",
                "a semicolon is used; start a second short sentence instead",
            );
        }
        for piece in parts.meanings.iter().map(|line| line.note.as_str()).chain(
            parts
                .sections()
                .into_iter()
                .flat_map(|(_, value, _)| sentences(value)),
        ) {
            if self.local("opener", piece.trim()) {
                self.fail("opener", format!("«{}» starts with an empty opener; start with the place, the occasion or the fact", piece.trim().chars().take(50).collect::<String>()));
            }
        }
        let prose = parts
            .meanings
            .iter()
            .flat_map(|line| [line.head.as_str(), &line.note])
            .chain([parts.literal.as_str(), &parts.nuance.why])
            .chain(parts.sections().into_iter().map(|(_, text, _)| text))
            .collect::<Vec<_>>()
            .join("\n");
        let terms = unique(
            self.local_hits("grammar", &prose)
                .into_iter()
                .map(|hit| hit.trim().to_lowercase())
                .collect(),
        );
        if !terms.is_empty() {
            self.fail(
                "grammar",
                format!(
                    "grammar terms are used ({}); put two real phrases side by side instead",
                    terms.join(", ")
                ),
            );
        }
        if matches(r"[()\[\]{}]", whole) {
            self.fail(
                "brackets",
                "brackets are used; say it in plain words or drop it",
            );
        }
        if parts.strings().iter().any(|value| matches("[*_#`]", value)) {
            self.fail("markup", "a string carries markup characters");
        }
        if parts.strings().iter().any(|value| value.contains('\n')) {
            self.fail("form", "a string carries a line break");
        }
        let [open, close] = &self.language.context.quotes;
        let stripped = whole.replace(open, "").replace(close, "");
        if matches("[\"“”]", &stripped) {
            self.fail(
                "quotation marks",
                "quotation marks of another language are used; use the reader's quotation marks",
            );
        }
        let no_latin = regex("[A-Za-z][A-Za-z' -]*")
            .replace_all(&prose, " ")
            .to_string();
        if let Some(hit) = self.local_hits("polite", &no_latin).first() {
            self.fail(
                "formal address",
                format!(
                    "«{}» addresses the reader as «вы»; use «ты» or say «говорят»",
                    hit.trim()
                ),
            );
        }
        for (name, rule, message) in [
            ("office", "clumsy", "office words are used"),
            (
                "standin",
                "clumsy",
                "a vague stand-in is used instead of the real word",
            ),
            ("narrated", "clumsy", "grammar is told in everyday words"),
        ] {
            let hits = unique(
                self.local_hits(name, &prose)
                    .into_iter()
                    .map(|hit| hit.trim().to_lowercase())
                    .collect(),
            );
            if !hits.is_empty() {
                self.fail(
                    rule,
                    format!(
                        "{message} ({}); put two real phrases side by side",
                        hits.join(", ")
                    ),
                );
            }
        }
        let said = [
            (
                "the first note",
                parts.meanings.first().map_or("", |line| line.note.as_str()),
            ),
            ("\"where\"", &parts.r#where.text),
            ("\"limits\"", &parts.limits.text),
            ("\"nuance\"", &parts.nuance.text),
        ];
        for (index, (name, value)) in said.iter().enumerate() {
            if value.trim().is_empty() {
                continue;
            }
            for (other_name, other) in said
                .iter()
                .skip(index + 1)
                .filter(|(_, value)| !value.trim().is_empty())
            {
                let pieces = sentences(value)
                    .into_iter()
                    .chain([*value])
                    .collect::<Vec<_>>();
                let others = sentences(other)
                    .into_iter()
                    .chain([*other])
                    .collect::<Vec<_>>();
                if pieces
                    .iter()
                    .any(|piece| others.iter().any(|other| shared(piece, other, &card.term)))
                {
                    self.fail("repeat", format!("{name} and {other_name} say the same thing; keep it in one place and drop it from the other"));
                }
            }
        }
    }

    fn form(&mut self, card: &Card, parts: &Parts, blocks: &[Block]) {
        for line in &parts.meanings {
            if super::layout::terminated(line.note.trim()) {
                self.fail(
                    "form",
                    format!("the note of the line «{}» ends with a full stop", line.head),
                );
            }
        }
        let [open, close] = &self.language.context.quotes;
        if !parts.literal.trim().is_empty()
            && (!(parts.literal.contains(open) && parts.literal.contains(close))
                || super::layout::terminated(parts.literal.trim()))
        {
            self.fail("form", "\"literal\" must give the picture in quotation marks and must not end with a full stop");
        }
        for (name, value, _) in parts.sections() {
            let value = value.trim();
            if value.is_empty() {
                continue;
            }
            if !super::layout::terminated(value) && !value.ends_with('»') && !value.ends_with(close)
            {
                self.fail(
                    "form",
                    format!("the text of \"{name}\" must end as a full sentence"),
                );
            }
            if matches(r"^\s*[—–-]\s|^\s*\d{1,3}[.)]\s", value) {
                self.fail(
                    "form",
                    format!("the text of \"{name}\" opens with a dash or a number"),
                );
            }
            if matches(r"[?!]\s*[,.:]|[?!]\s+(?:а не|но не|или)\s", value) {
                self.fail("form", format!("in the text of \"{name}\" a learning phrase carries its own question or exclamation mark and the marks pile up; drop that mark"));
            }
            if matches(
                r"[A-Za-z][A-Za-z',.!? -]{6,}\s[—–]\s«|[A-Za-z][A-Za-z'’-]*(?:\s+[A-Za-z][A-Za-z'’-]*){6,}",
                value,
            ) && !self.language.known.contains(&unicode_script::Script::Latin)
            {
                self.fail("example", format!("the text of \"{name}\" holds a whole sentence in the learning language or its translation; move it to \"example\""));
            }
            for sentence in sentences(value) {
                let count = hits(r"[\p{L}][\p{L}'’-]*", sentence).len();
                if count > 18 {
                    self.fail("clumsy", format!("the sentence «{}…» of \"{name}\" has {count} words, the limit is 18; split it or cut it", sentence.chars().take(60).collect::<String>()));
                }
            }
        }
        let examples = parts
            .sections()
            .into_iter()
            .filter(|(_, _, example)| !example.text.trim().is_empty())
            .collect::<Vec<_>>();
        let pairs = blocks
            .windows(2)
            .filter(|pair| {
                matches!(pair[0], Block::Paragraph(_))
                    && text(&pair[0]).starts_with(open)
                    && matches(r"^\s*[—–]\s", &text(&pair[1]))
            })
            .collect::<Vec<_>>();
        if pairs.len() != examples.len()
            || pairs.iter().any(|pair| {
                chunks(&pair[0])
                    .iter()
                    .any(|chunk| chunk.bold || chunk.italic)
                    || !text(&pair[0]).ends_with(close)
            })
        {
            self.fail(
                "example",
                "an example on the card is not a plain phrase with its translation under it",
            );
        }
        for (_, _, example) in examples {
            if bare(&example.text) == bare(&card.sentence) {
                self.fail("form", "an example repeats the sentence of the card");
            }
            if matches("[*_«»\"“”]", &example.text)
                || matches("[*_]", &example.translation)
                || example.text.contains(open)
                || example.text.contains(close)
            {
                self.fail("example", "an example carries bold marks or quotation marks of its own; write it bare, the app puts the phrase in quotes and never prints it bold");
            }
            if matches("^[—–-]|^[«\"“]", example.text.trim())
                || matches("^[—–-]", example.translation.trim())
            {
                self.fail(
                    "form",
                    "an example carries its own dash or quotation marks; write it bare",
                );
            }
        }
    }
}

/// Review every rule against the exact strings that the card will render.
#[must_use]
pub fn judge(card: &Card, parts: &Parts, language: &Language) -> Verdict {
    let markdown = compose(parts, language);
    let blocks = parse_markdown(&markdown);
    let whole = blocks.iter().map(text).collect::<Vec<_>>().join("\n");
    let mut review = Review {
        language,
        fails: Vec::new(),
    };
    review.list(card, parts);
    if parts.meanings.is_empty() {
        review.fail("list", "\"meanings\" is empty");
        return Verdict {
            fails: review.fails,
            markdown,
        };
    }
    review.structure(parts, &blocks);
    review.quotes(parts, &blocks);
    review.empty(parts, &whole);
    review.lengths(parts, &whole);
    review.why(parts);
    review.wording(card, parts, &whole);
    review.form(card, parts, &blocks);
    Verdict {
        fails: review.fails,
        markdown,
    }
}

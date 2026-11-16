//! Deterministic normalization, quotation, and four-section explanation layout.

use super::{Example, Language, Parts};
use unicode_segmentation::UnicodeSegmentation;

/// Trim model strings and complete prose sentences without changing their facts.
#[must_use]
pub fn tidy(parts: &Parts) -> Parts {
    let mut output = parts.clone();
    for line in &mut output.meanings {
        line.head = line.head.trim().to_string();
        line.note = line.note.trim().to_string();
        line.from = line.from.trim().to_string();
    }
    output.literal = output.literal.trim().to_string();
    output.r#where.text = sentence(&output.r#where.text);
    output.limits.text = sentence(&output.limits.text);
    output.nuance.text = sentence(&output.nuance.text);
    output.nuance.why = sentence(&output.nuance.why);
    for example in [
        &mut output.r#where.example,
        &mut output.limits.example,
        &mut output.nuance.example,
    ] {
        example.text = example.text.trim().to_string();
        example.translation = example.translation.trim().to_string();
    }
    output
}

fn sentence(value: &str) -> String {
    let text = value.trim();
    if text.is_empty() || terminated(text) || text.ends_with(['…', '»']) {
        text.to_string()
    } else {
        format!("{text}.")
    }
}

/// Recognize complete sentence punctuation shared by normalization and review.
pub(super) fn terminated(text: &str) -> bool {
    text.ends_with(['.', '!', '?', '。', '！', '？', '؟', '।', '॥'])
}

/// Replace model markers and quote foreign-script word runs outside existing quotes.
#[must_use]
pub fn quote(text: &str, language: &Language) -> String {
    let [open, close] = &language.context.quotes;
    let marked = text.replace('⟦', open).replace('⟧', close);
    let mut output = String::new();
    let mut remaining = marked.as_str();
    while let Some(start) = remaining.find(open) {
        output.push_str(&runs(&remaining[..start], language));
        let after = start + open.len();
        if let Some(end) = remaining[after..].find(close) {
            let end = after + end + close.len();
            output.push_str(&remaining[start..end]);
            remaining = &remaining[end..];
        } else {
            output.push_str(&remaining[start..]);
            return output;
        }
    }
    output.push_str(&runs(remaining, language));
    output
}

fn runs(text: &str, language: &Language) -> String {
    let chars = text.graphemes(true).collect::<Vec<_>>();
    let mut output = String::new();
    let mut index = 0;
    while index < chars.len() {
        if !chars[index].chars().any(|ch| language.foreign(ch)) {
            output.push_str(chars[index]);
            index += 1;
            continue;
        }
        let start = index;
        index = word(&chars, index, language);
        loop {
            let mut next = index;
            while next < chars.len() && chars[next].chars().all(char::is_whitespace) {
                next += 1;
            }
            if next == index {
                break;
            }
            let digit = next;
            while next < chars.len() && chars[next].chars().all(|ch| ch.is_ascii_digit()) {
                next += 1;
            }
            if next > digit {
                let space = next;
                while next < chars.len() && chars[next].chars().all(char::is_whitespace) {
                    next += 1;
                }
                if space == next {
                    break;
                }
            }
            if next >= chars.len() || !chars[next].chars().any(|ch| language.foreign(ch)) {
                break;
            }
            index = word(&chars, next, language);
        }
        output.push_str(&language.context.quotes[0]);
        output.push_str(&chars[start..index].concat());
        output.push_str(&language.context.quotes[1]);
    }
    output
}

fn word(chars: &[&str], start: usize, language: &Language) -> usize {
    let belongs = |text: &str| {
        text.chars().any(|ch| language.foreign(ch)) || text.chars().all(|ch| ch.is_ascii_digit())
    };
    let mut index = start + 1;
    while index < chars.len() {
        if belongs(chars[index]) {
            index += 1;
        } else if matches!(chars[index], "'" | "’" | "-")
            && chars.get(index + 1).is_some_and(|next| belongs(next))
        {
            index += 2;
        } else {
            break;
        }
    }
    index
}

/// Compose only nonempty sections, using the reader's headings and quotation marks.
#[must_use]
pub fn compose(parts: &Parts, language: &Language) -> String {
    let mut list = parts
        .meanings
        .iter()
        .map(|line| {
            format!(
                "- {} — {}",
                quote(line.head.trim(), language),
                quote(line.note.trim(), language)
            )
        })
        .collect::<Vec<_>>();
    if !parts.literal.trim().is_empty() {
        list.insert(
            1.min(list.len()),
            format!(
                "- {} — {}",
                language.context.literally,
                quote(parts.literal.trim(), language)
            ),
        );
    }
    let mut blocks = vec![format!(
        "**{}**\n{}",
        language.context.headers[0],
        list.join("\n")
    )];
    for (index, (_, text, example)) in parts.sections().into_iter().enumerate() {
        if text.trim().is_empty() {
            continue;
        }
        let why = if index == 2 && !parts.nuance.why.trim().is_empty() {
            format!(" {}", quote(parts.nuance.why.trim(), language))
        } else {
            String::new()
        };
        blocks.push(format!(
            "**{}**\n{}{why}",
            language.context.headers[index + 1],
            quote(text.trim(), language)
        ));
        if !example.text.trim().is_empty() {
            blocks.push(example_block(example, language));
        }
    }
    blocks.join("\n\n")
}

fn example_block(example: &Example, language: &Language) -> String {
    format!(
        "{}{}{}\n\n— {}",
        language.context.quotes[0],
        example.text.trim(),
        language.context.quotes[1],
        example.translation.trim()
    )
}

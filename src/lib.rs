// Native (Rust + N-API) drop-in engine for makefun-site 1.0.0: solve_all(cases_json) parses the
// harness's cases.json text, answers every case with exactly makefun-site's output tree, and returns
// the answers.json text. Semantics port the reference implementation function by function.

use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use unicode_categories::UnicodeCategories;
use unicode_normalization::UnicodeNormalization;

// ---------------------------------------------------------------- text

fn escape_html_into(out: &mut String, text: &str) {
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(c),
        }
    }
}

fn escape_html(text: &str) -> String {
    if !text.bytes().any(|b| matches!(b, b'&' | b'<' | b'>' | b'"' | b'\'')) {
        return text.to_string();
    }
    let mut out = String::with_capacity(text.len() + 8);
    escape_html_into(&mut out, text);
    out
}

const SLUG_MAX: usize = 64;

fn slugify(text: &str) -> String {
    let mut out = String::new();
    let mut gap = false;
    for ch in text.nfkd() {
        if ch.is_mark() {
            continue;
        }
        let keep = if ch >= 'A' && ch <= 'Z' {
            Some(ch.to_ascii_lowercase())
        } else if (ch >= 'a' && ch <= 'z') || (ch >= '0' && ch <= '9') {
            Some(ch)
        } else {
            None
        };
        match keep {
            None => {
                gap = !out.is_empty();
            }
            Some(k) => {
                if gap {
                    out.push('-');
                }
                gap = false;
                out.push(k);
            }
        }
    }
    let truncated: String = out.chars().take(SLUG_MAX).collect();
    let trimmed = truncated.trim_end_matches('-');
    if trimmed.is_empty() {
        "untitled".to_string()
    } else {
        trimmed.to_string()
    }
}

fn unique_slug(slug: &str, taken: &mut HashSet<String>) -> String {
    let mut out = slug.to_string();
    let mut n = 2;
    while taken.contains(&out) {
        out = format!("{}-{}", slug, n);
        n += 1;
    }
    taken.insert(out.clone());
    out
}

const MONTHS: [&str; 12] = [
    "January", "February", "March", "April", "May", "June", "July", "August", "September", "October",
    "November", "December",
];

fn long_date(date: &str) -> String {
    let mut parts = date.split('-');
    let y = parts.next().unwrap_or("").to_string();
    let m: usize = parts.next().and_then(|s| s.parse().ok()).unwrap_or(1);
    let d: i64 = parts.next().and_then(|s| s.parse().ok()).unwrap_or(0);
    format!("{} {} {}", d, MONTHS[m.saturating_sub(1).min(11)], y)
}

// ---------------------------------------------------------------- front matter

#[derive(Default, Clone)]
struct Fm {
    title: Option<String>,
    date: Option<String>,
    tags: Option<Vec<String>>,
    draft: bool,
    layout: Option<String>,
    slug: Option<String>,
    summary: Option<String>,
}

fn days_in(y: i64, m: i64) -> i64 {
    if m == 2 {
        if (y % 4 == 0 && y % 100 != 0) || y % 400 == 0 {
            29
        } else {
            28
        }
    } else if m == 4 || m == 6 || m == 9 || m == 11 {
        30
    } else {
        31
    }
}

fn string_of(raw: &str) -> Result<String, String> {
    if raw.starts_with('"') {
        if raw.len() < 2 || !raw.ends_with('"') {
            return Err("a quoted string is not closed".to_string());
        }
        let inner = &raw[1..raw.len() - 1];
        let chars: Vec<char> = inner.chars().collect();
        let mut out = String::new();
        let mut i = 0;
        while i < chars.len() {
            let c = chars[i];
            if c == '\\' {
                if i + 1 >= chars.len() {
                    return Err("a quoted string holds an escape other than \\\" or \\\\".to_string());
                }
                let n = chars[i + 1];
                if n != '"' && n != '\\' {
                    return Err("a quoted string holds an escape other than \\\" or \\\\".to_string());
                }
                out.push(n);
                i += 2;
            } else if c == '"' {
                return Err("a quoted string holds a stray quote".to_string());
            } else {
                out.push(c);
                i += 1;
            }
        }
        return Ok(out);
    }
    if raw.starts_with('[') {
        return Err("a bare string opens on [".to_string());
    }
    Ok(raw.to_string())
}

enum FmVal {
    Str(String),
    Tags(Vec<String>),
    Bool(bool),
}

fn value_of(key: &str, raw: &str) -> Result<FmVal, String> {
    match key {
        "title" | "summary" => match string_of(raw) {
            Ok(v) => Ok(FmVal::Str(v)),
            Err(e) => Err(format!("key \"{}\": {}", key, e)),
        },
        "date" => {
            let b = raw.as_bytes();
            let ok_shape = b.len() == 10
                && b[4] == b'-'
                && b[7] == b'-'
                && b.iter().enumerate().all(|(i, c)| i == 4 || i == 7 || c.is_ascii_digit());
            let (y, mo, d) = if ok_shape {
                (
                    raw[0..4].parse::<i64>().unwrap_or(0),
                    raw[5..7].parse::<i64>().unwrap_or(0),
                    raw[8..10].parse::<i64>().unwrap_or(0),
                )
            } else {
                (0, 0, 0)
            };
            if !ok_shape || y < 1970 || y > 2099 || mo < 1 || mo > 12 || d < 1 || d > days_in(y, mo) {
                return Err("key \"date\": not a date from 1970-01-01 to 2099-12-31 (YYYY-MM-DD)".to_string());
            }
            Ok(FmVal::Str(raw.to_string()))
        }
        "tags" => {
            if !raw.starts_with('[') || !raw.ends_with(']') {
                return Err("key \"tags\": not a list [a, b]".to_string());
            }
            let inner = &raw[1..raw.len() - 1];
            if inner.trim().is_empty() {
                return Ok(FmVal::Tags(vec![]));
            }
            let tags: Vec<String> = inner.split(',').map(|t| t.trim().to_string()).collect();
            if tags.iter().any(|t| t.is_empty() || t.chars().any(|c| c == '"' || c == '[' || c == ']')) {
                return Err("key \"tags\": an empty tag, or one holding \" [ or ]".to_string());
            }
            Ok(FmVal::Tags(tags))
        }
        "draft" => {
            if raw == "true" {
                Ok(FmVal::Bool(true))
            } else if raw == "false" {
                Ok(FmVal::Bool(false))
            } else {
                Err("key \"draft\": true or false".to_string())
            }
        }
        "layout" => {
            if !raw.is_empty() && raw.bytes().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-') {
                Ok(FmVal::Str(raw.to_string()))
            } else {
                Err("key \"layout\": not a template name".to_string())
            }
        }
        "slug" => {
            let ok = !raw.is_empty()
                && raw.len() <= 64
                && raw.split('-').all(|seg| {
                    !seg.is_empty() && seg.bytes().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
                });
            if ok {
                Ok(FmVal::Str(raw.to_string()))
            } else {
                Err("key \"slug\": not a slug".to_string())
            }
        }
        _ => Err(format!("unknown key \"{}\"", key)),
    }
}

/// Parse one "key: value" line: (key, raw-with-leading-space-stripped) or None when not of that shape.
fn fm_line(line: &str) -> Option<(String, Option<String>)> {
    let b = line.as_bytes();
    let mut i = 0;
    while i < b.len() && (b[i].is_ascii_lowercase() || b[i] == b'_') {
        i += 1;
    }
    if i == 0 || i >= b.len() || b[i] != b':' {
        return None;
    }
    let key = line[0..i].to_string();
    let rest = &line[i + 1..];
    if rest.is_empty() {
        return Some((key, None));
    }
    if !rest.starts_with(' ') {
        return None;
    }
    Some((key, Some(rest[1..].to_string())))
}

fn parse_front_matter(text: &str) -> Result<(Fm, String), String> {
    let nl = text.find('\n');
    let first = match nl {
        Some(p) => &text[..p],
        None => text,
    };
    if first != "---" {
        return Err("no front matter".to_string());
    }
    if nl.is_none() {
        return Err("the front matter is not closed".to_string());
    }
    let mut head: Vec<&str> = Vec::new();
    let mut pos = nl.unwrap() + 1;
    let mut close_end: Option<usize> = None;
    loop {
        let e = text[pos..].find('\n').map(|o| pos + o);
        let line = match e {
            Some(end) => &text[pos..end],
            None => &text[pos..],
        };
        if line == "---" {
            close_end = Some(match e {
                Some(end) => end + 1,
                None => text.len(),
            });
            break;
        }
        head.push(line);
        match e {
            Some(end) => pos = end + 1,
            None => break,
        }
    }
    let close_end = match close_end {
        Some(c) => c,
        None => return Err("the front matter is not closed".to_string()),
    };
    let mut data = Fm::default();
    let mut seen: HashSet<String> = HashSet::new();
    for (h, line) in head.iter().enumerate() {
        if line.bytes().all(|c| c == b' ' || c == b'\t') || line.starts_with('#') {
            continue;
        }
        let (key, raw_opt) = match fm_line(line) {
            Some(v) => v,
            None => return Err(format!("line {} is not \"key: value\"", h + 2)),
        };
        let raw_full = raw_opt.unwrap_or_default();
        let raw = raw_full.trim_end_matches(|c| c == ' ' || c == '\t');
        if !matches!(key.as_str(), "title" | "date" | "tags" | "draft" | "layout" | "slug" | "summary") {
            return Err(format!("unknown key \"{}\"", key));
        }
        if !seen.insert(key.clone()) {
            return Err(format!("key \"{}\" is given twice", key));
        }
        if raw.is_empty() {
            return Err(format!("key \"{}\" has no value", key));
        }
        match value_of(&key, raw)? {
            FmVal::Str(v) => match key.as_str() {
                "title" => data.title = Some(v),
                "date" => data.date = Some(v),
                "layout" => data.layout = Some(v),
                "slug" => data.slug = Some(v),
                "summary" => data.summary = Some(v),
                _ => unreachable!(),
            },
            FmVal::Tags(v) => data.tags = Some(v),
            FmVal::Bool(v) => data.draft = v,
        }
    }
    if data.title.is_none() {
        return Err("no title".to_string());
    }
    Ok((data, text[close_end..].to_string()))
}

// ---------------------------------------------------------------- markdown

fn is_blank(line: &str) -> bool {
    line.bytes().all(|c| c == b' ' || c == b'\t')
}

fn fence_open(line: &str) -> Option<String> {
    let b = line.as_bytes();
    if b.len() < 3 || b[0] != b'`' || b[1] != b'`' || b[2] != b'`' {
        return None;
    }
    let mut i = 3;
    while i < b.len() {
        let c = b[i];
        if c.is_ascii_alphanumeric() || c == b'_' || c == b'+' || c == b'-' {
            i += 1;
        } else {
            break;
        }
    }
    let lang = line[3..i].to_string();
    while i < b.len() {
        let c = b[i];
        if c == b' ' || c == b'\t' {
            i += 1;
        } else {
            return None;
        }
    }
    Some(lang)
}

fn fence_close(line: &str) -> bool {
    let b = line.as_bytes();
    if b.len() < 3 || b[0] != b'`' || b[1] != b'`' || b[2] != b'`' {
        return false;
    }
    b[3..].iter().all(|&c| c == b' ' || c == b'\t')
}

fn heading_parse(line: &str) -> Option<(usize, String)> {
    let b = line.as_bytes();
    let mut hashes = 0;
    while hashes < b.len() && b[hashes] == b'#' {
        hashes += 1;
    }
    if hashes == 0 || hashes > 6 {
        return None;
    }
    if hashes >= b.len() || (b[hashes] != b' ' && b[hashes] != b'\t') {
        return None;
    }
    let mut i = hashes;
    while i < b.len() && (b[i] == b' ' || b[i] == b'\t') {
        i += 1;
    }
    let mut end = b.len();
    while end > i && (b[end - 1] == b' ' || b[end - 1] == b'\t') {
        end -= 1;
    }
    if end == i {
        return None;
    }
    Some((hashes, line[i..end].to_string()))
}

fn is_hr(line: &str) -> bool {
    let b = line.as_bytes();
    let mut end = b.len();
    while end > 0 && (b[end - 1] == b' ' || b[end - 1] == b'\t') {
        end -= 1;
    }
    if end != 3 {
        return false;
    }
    (b[0] == b'-' && b[1] == b'-' && b[2] == b'-') || (b[0] == b'*' && b[1] == b'*' && b[2] == b'*')
}

fn bullet_of(line: &str) -> Option<String> {
    let b = line.as_bytes();
    if b.len() >= 2 && (b[0] == b'-' || b[0] == b'*') && b[1] == b' ' {
        Some(line[2..].to_string())
    } else {
        None
    }
}

fn ordered_of(line: &str) -> Option<(i64, String)> {
    let b = line.as_bytes();
    let mut i = 0;
    while i < b.len() && b[i].is_ascii_digit() {
        i += 1;
    }
    if i == 0 || i > 9 {
        return None;
    }
    if i + 1 >= b.len() || b[i] != b'.' || b[i + 1] != b' ' {
        return None;
    }
    Some((line[0..i].parse().unwrap_or(0), line[i + 2..].to_string()))
}

fn is_continued(line: &str) -> bool {
    let b = line.as_bytes();
    let mut i = 0;
    while i < b.len() && b[i] == b' ' {
        i += 1;
    }
    if i < 2 || i >= b.len() {
        return false;
    }
    let c = b[i];
    c != b' ' && c != b'\t' && c != 0x0b && c != 0x0c && c != b'\r'
}

fn trim_line(line: &str) -> String {
    line.trim_matches(|c| c == ' ' || c == '\t').to_string()
}

fn ends_paragraph(line: &str) -> bool {
    is_blank(line) || fence_open(line).is_some() || heading_parse(line).is_some() || is_hr(line)
}

enum Tok {
    Heading { depth: usize, text: String },
    Code { lang: String, lines: Vec<String> },
    Hr,
    Blockquote { lines: Vec<String> },
    List { ordered: bool, start: i64, items: Vec<String> },
    Paragraph { text: String },
}

fn lex(src: &str) -> Vec<Tok> {
    let lines: Vec<&str> = src.split('\n').collect();
    let mut tokens = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i];
        if is_blank(line) {
            i += 1;
            continue;
        }
        if let Some(fence) = fence_open(line) {
            let mut body = Vec::new();
            i += 1;
            while i < lines.len() && !fence_close(lines[i]) {
                body.push(lines[i].to_string());
                i += 1;
            }
            i += 1;
            tokens.push(Tok::Code { lang: fence, lines: body });
            continue;
        }
        if let Some((depth, text)) = heading_parse(line) {
            tokens.push(Tok::Heading { depth, text });
            i += 1;
            continue;
        }
        if is_hr(line) {
            tokens.push(Tok::Hr);
            i += 1;
            continue;
        }
        if line.starts_with('>') {
            let mut body = Vec::new();
            while i < lines.len() && lines[i].starts_with('>') {
                let l = lines[i];
                let stripped = if let Some(rest) = l.strip_prefix("> ") {
                    rest
                } else {
                    &l[1..]
                };
                body.push(stripped.to_string());
                i += 1;
            }
            let cleaned: Vec<String> = body.iter().map(|l| trim_line(l)).filter(|l| !l.is_empty()).collect();
            tokens.push(Tok::Blockquote { lines: cleaned });
            continue;
        }
        let bullet = bullet_of(line);
        let ordered = if bullet.is_none() { ordered_of(line) } else { None };
        if bullet.is_some() || ordered.is_some() {
            let is_ordered = ordered.is_some();
            let mut items: Vec<Vec<String>> = Vec::new();
            while i < lines.len() {
                let m = if is_ordered { ordered_of(lines[i]).map(|(_, t)| t) } else { bullet_of(lines[i]) };
                if let Some(t) = m {
                    items.push(vec![trim_line(&t)]);
                    i += 1;
                } else if is_continued(lines[i]) {
                    items.last_mut().unwrap().push(trim_line(lines[i]));
                    i += 1;
                } else {
                    break;
                }
            }
            tokens.push(Tok::List {
                ordered: is_ordered,
                start: if is_ordered { ordered.unwrap().0 } else { 1 },
                items: items.iter().map(|parts| parts.join("\n")).collect(),
            });
            continue;
        }
        let mut body = vec![trim_line(line)];
        i += 1;
        while i < lines.len() && !ends_paragraph(lines[i]) {
            body.push(trim_line(lines[i]));
            i += 1;
        }
        tokens.push(Tok::Paragraph { text: body.join("\n") });
    }
    tokens
}

fn escapable(c: u8) -> bool {
    matches!(c, b'\\' | b'`' | b'*' | b'_' | b'[' | b']' | b'(' | b')' | b'#' | b'+' | b'-' | b'.' | b'!' | b'>')
}

fn find_unescaped(s: &[u8], what: &[u8], from: usize, hi: usize) -> Option<usize> {
    let wlen = what.len();
    if hi < wlen {
        return None;
    }
    let mut j = from;
    while j <= hi - wlen {
        if s[j] == b'\\' && j + 1 < hi && escapable(s[j + 1]) {
            j += 2;
            continue;
        }
        if &s[j..j + wlen] == what {
            return Some(j);
        }
        j += 1;
    }
    None
}

fn find_single_star(s: &[u8], from: usize, hi: usize) -> Option<usize> {
    let mut j = from;
    while j < hi {
        let c = s[j];
        if c == b'\\' && j + 1 < hi && escapable(s[j + 1]) {
            j += 2;
            continue;
        }
        if c != b'*' {
            j += 1;
            continue;
        }
        if j + 1 < hi && s[j + 1] == b'*' {
            j += 2;
            continue;
        }
        return Some(j);
    }
    None
}

struct LinkEnv<'a> {
    resolve: &'a dyn Fn(&str) -> String,
}

fn inline_range(s: &str, lo: usize, hi: usize, env: &LinkEnv, out: &mut String) {
    let b = s.as_bytes();
    let mut i = lo;
    while i < hi {
        let mut j = i;
        while j < hi {
            let code = b[j];
            if matches!(code, b'\\' | b'`' | b'*' | b'[' | b'&' | b'<' | b'>' | b'"' | b'\'') {
                break;
            }
            j += 1;
        }
        if j > i {
            out.push_str(&s[i..j]);
            i = j;
            if i >= hi {
                break;
            }
        }
        let c = b[i];
        match c {
            b'\\' => {
                if i + 1 < hi && escapable(b[i + 1]) {
                    escape_html_into(out, &s[i + 1..i + 2]);
                    i += 2;
                } else {
                    out.push('\\');
                    i += 1;
                }
            }
            b'`' => {
                let k = s[i + 1..hi].find('`').map(|o| i + 1 + o);
                if let Some(k) = k {
                    if k > i + 1 {
                        out.push_str("<code>");
                        escape_html_into(out, &s[i + 1..k]);
                        out.push_str("</code>");
                        i = k + 1;
                        continue;
                    }
                }
                out.push('`');
                i += 1;
            }
            b'*' => {
                if i + 1 < hi && b[i + 1] == b'*' {
                    match find_unescaped(b, b"**", i + 2, hi) {
                        Some(k) if k > i + 2 => {
                            out.push_str("<strong>");
                            inline_range(s, i + 2, k, env, out);
                            out.push_str("</strong>");
                            i = k + 2;
                        }
                        _ => {
                            out.push_str("**");
                            i += 2;
                        }
                    }
                    continue;
                }
                match find_single_star(b, i + 1, hi) {
                    Some(k) if k > i + 1 => {
                        out.push_str("<em>");
                        inline_range(s, i + 1, k, env, out);
                        out.push_str("</em>");
                        i = k + 1;
                    }
                    _ => {
                        out.push('*');
                        i += 1;
                    }
                }
            }
            b'[' => {
                let k = find_unescaped(b, b"]", i + 1, hi);
                let mut m: Option<usize> = None;
                if let Some(k) = k {
                    if k + 1 < hi && b[k + 1] == b'(' {
                        if let Some(p) = s[k + 2..hi].find(')') {
                            m = Some(k + 2 + p);
                        }
                    }
                }
                let target = match (k, m) {
                    (Some(k), Some(m)) => &s[k + 2..m],
                    _ => "",
                };
                if !target.is_empty() && !target.chars().any(|c| c.is_whitespace()) {
                    out.push_str("<a href=\"");
                    escape_html_into(out, &(env.resolve)(target));
                    out.push_str("\">");
                    inline_range(s, i + 1, k.unwrap(), env, out);
                    out.push_str("</a>");
                    i = m.unwrap() + 1;
                    continue;
                }
                out.push('[');
                i += 1;
            }
            _ => {
                // one of & < > " '
                escape_html_into(out, &s[i..i + 1]);
                i += 1;
            }
        }
    }
}

fn inline(s: &str, env: &LinkEnv) -> String {
    let mut out = String::with_capacity(s.len() + 16);
    inline_range(s, 0, s.len(), env, &mut out);
    out
}

struct TocItem {
    level: usize,
    id: String,
    html: String,
}

fn render_markdown(
    src: &str,
    env: &LinkEnv,
    slug_cache: &mut HashMap<String, String>,
) -> (String, Vec<TocItem>, String) {
    let mut ids: HashSet<String> = HashSet::new();
    let mut blocks: Vec<String> = Vec::new();
    let mut toc: Vec<TocItem> = Vec::new();
    let mut excerpt: Option<String> = None;
    for t in lex(src) {
        match t {
            Tok::Heading { depth, text } => {
                let base = match slug_cache.get(&text) {
                    Some(v) => v.clone(),
                    None => {
                        let v = slugify(&text);
                        slug_cache.insert(text.clone(), v.clone());
                        v
                    }
                };
                let id = unique_slug(&base, &mut ids);
                let html = inline(&text, env);
                blocks.push(format!("<h{} id=\"{}\">{}</h{}>", depth, id, html, depth));
                if depth == 2 || depth == 3 {
                    toc.push(TocItem { level: depth, id, html });
                }
            }
            Tok::Code { lang, lines } => {
                let mut block = String::from("<pre><code");
                if !lang.is_empty() {
                    block.push_str(" class=\"language-");
                    escape_html_into(&mut block, &lang);
                    block.push('"');
                }
                block.push('>');
                for l in &lines {
                    escape_html_into(&mut block, l);
                    block.push('\n');
                }
                block.push_str("</code></pre>");
                blocks.push(block);
            }
            Tok::Hr => blocks.push("<hr>".to_string()),
            Tok::Blockquote { lines } => {
                if lines.is_empty() {
                    blocks.push("<blockquote>\n</blockquote>".to_string());
                } else {
                    blocks.push(format!("<blockquote>\n<p>{}</p>\n</blockquote>", inline(&lines.join("\n"), env)));
                }
            }
            Tok::List { ordered, start, items } => {
                let tag = if ordered { "ol" } else { "ul" };
                let mut block = if ordered && start != 1 {
                    format!("<ol start=\"{}\">", start)
                } else {
                    format!("<{}>", tag)
                };
                block.push('\n');
                for item in &items {
                    block.push_str("<li>");
                    block.push_str(&inline(item, env));
                    block.push_str("</li>\n");
                }
                block.push_str(&format!("</{}>", tag));
                blocks.push(block);
            }
            Tok::Paragraph { text } => {
                let html = inline(&text, env);
                if excerpt.is_none() {
                    excerpt = Some(html.clone());
                }
                blocks.push(format!("<p>{}</p>", html));
            }
        }
    }
    let mut html = String::new();
    for b in &blocks {
        html.push_str(b);
        html.push('\n');
    }
    (html, toc, excerpt.unwrap_or_default())
}

// ---------------------------------------------------------------- templates

#[derive(Clone)]
enum Val {
    None,
    Str(String),
    Num(i64),
    Bool(bool),
    Arr(Vec<Val>),
    Obj(Rc<Vec<(String, Val)>>),
}

fn obj(pairs: Vec<(String, Val)>) -> Val {
    Val::Obj(Rc::new(pairs))
}

fn text_of(v: &Val) -> String {
    match v {
        Val::Str(s) => s.clone(),
        Val::Num(n) => n.to_string(),
        Val::Bool(b) => if *b { "true".to_string() } else { "false".to_string() },
        Val::Arr(xs) => xs.iter().map(text_of).collect::<Vec<_>>().join(", "),
        _ => String::new(),
    }
}

fn truthy(v: Option<&Val>) -> bool {
    match v {
        None => false,
        Some(Val::None) => false,
        Some(Val::Bool(b)) => *b,
        Some(Val::Str(s)) => !s.is_empty(),
        Some(Val::Num(n)) => *n != 0,
        Some(Val::Arr(xs)) => !xs.is_empty(),
        Some(Val::Obj(_)) => true,
    }
}

/// Scope chain: a stack of frames; lookup walks from the top frame down.
type Scope = Vec<HashMap<String, Val>>;

fn lookup(scope: &Scope, path: &[String]) -> Option<Val> {
    let mut found: Option<Val> = None;
    for frame in scope.iter().rev() {
        if let Some(v) = frame.get(&path[0]) {
            found = Some(v.clone());
            break;
        }
    }
    let mut v = found?;
    for seg in &path[1..] {
        match &v {
            Val::Obj(pairs) => {
                let mut next = None;
                for (k, val) in pairs.iter() {
                    if k == seg {
                        next = Some(val.clone());
                        break;
                    }
                }
                v = next?;
            }
            Val::Arr(xs) => {
                if seg == "length" {
                    v = Val::Num(xs.len() as i64);
                } else {
                    return None;
                }
            }
            _ => return None,
        }
    }
    Some(v)
}

enum Node {
    Text(String),
    Var(Vec<String>),
    Raw(Vec<String>),
    If { not: bool, path: Vec<String>, then: Vec<Node>, els: Option<Vec<Node>> },
    For { name: String, path: Vec<String>, body: Vec<Node> },
    Include(String),
}

struct TemplateError(String);

fn is_ident(s: &str) -> bool {
    let b = s.as_bytes();
    if b.is_empty() || !(b[0].is_ascii_alphabetic() || b[0] == b'_') {
        return false;
    }
    b.iter().all(|c| c.is_ascii_alphanumeric() || *c == b'_')
}

fn path_of(name: &str, text: &str) -> Result<Vec<String>, TemplateError> {
    let parts: Vec<String> = text.split('.').map(|s| s.to_string()).collect();
    if parts.iter().any(|p| !is_ident(p)) {
        return Err(TemplateError(format!("template \"{}\": {:?} is not a path", name, text)));
    }
    Ok(parts)
}

enum TokT {
    Text(String),
    Var(Vec<String>),
    Raw(Vec<String>),
    Tag(String),
}

fn tokenize(name: &str, src: &str) -> Result<Vec<TokT>, TemplateError> {
    let b = src.as_bytes();
    let mut tokens = Vec::new();
    let mut text = String::new();
    let mut i = 0usize;
    while i < src.len() {
        let open = match src[i..].find('{') {
            Some(o) => i + o,
            None => {
                text.push_str(&src[i..]);
                break;
            }
        };
        if open == src.len() - 1 {
            text.push_str(&src[i..]);
            break;
        }
        let next = b[open + 1];
        if next != b'{' && next != b'%' {
            text.push_str(&src[i..open + 1]);
            i = open + 1;
            continue;
        }
        text.push_str(&src[i..open]);
        if next == b'{' {
            let raw = open + 2 < src.len() && b[open + 2] == b'{';
            let closer = if raw { "}}}" } else { "}}" };
            let start = open + if raw { 3 } else { 2 };
            let close = match src[start..].find(closer) {
                Some(o) => start + o,
                None => {
                    return Err(TemplateError(format!(
                        "template \"{}\": a {} is not closed",
                        name,
                        if raw { "{{{" } else { "{{" }
                    )))
                }
            };
            if !text.is_empty() {
                tokens.push(TokT::Text(std::mem::take(&mut text)));
            }
            let inner = src[start..close].trim();
            tokens.push(if raw { TokT::Raw(path_of(name, inner)?) } else { TokT::Var(path_of(name, inner)?) });
            i = close + closer.len();
            continue;
        }
        // {% ... %}
        let close = match src[open + 2..].find("%}") {
            Some(o) => open + 2 + o,
            None => return Err(TemplateError(format!("template \"{}\": a {{% is not closed", name))),
        };
        let end = close + 2;
        let line_start = src[..open].rfind('\n').map(|p| p + 1).unwrap_or(0);
        let nl = src[end..].find('\n').map(|o| end + o);
        let line_end = nl.unwrap_or(src.len());
        let standalone = src[line_start..open].bytes().all(|c| c == b' ' || c == b'\t')
            && src[end..line_end].bytes().all(|c| c == b' ' || c == b'\t');
        if standalone {
            let keep = open - line_start;
            let cur = text.len();
            text.truncate(cur - keep);
        }
        if !text.is_empty() {
            tokens.push(TokT::Text(std::mem::take(&mut text)));
        }
        tokens.push(TokT::Tag(src[open + 2..close].trim().to_string()));
        i = if standalone {
            match nl {
                Some(p) => p + 1,
                None => src.len(),
            }
        } else {
            end
        };
    }
    if !text.is_empty() {
        tokens.push(TokT::Text(text));
    }
    Ok(tokens)
}

enum Frame {
    Root(Vec<Node>),
    For { name: String, path: Vec<String>, body: Vec<Node> },
    If { not: bool, path: Vec<String>, then: Vec<Node>, els: Option<Vec<Node>>, in_else: bool },
}

fn parse_template(name: &str, src: &str) -> Result<Vec<Node>, TemplateError> {
    let tokens = tokenize(name, src)?;
    let mut stack: Vec<Frame> = vec![Frame::Root(Vec::new())];
    fn cur_body(stack: &mut [Frame]) -> &mut Vec<Node> {
        match stack.last_mut().unwrap() {
            Frame::Root(b) => b,
            Frame::For { body, .. } => body,
            Frame::If { then, els, in_else, .. } => {
                if *in_else {
                    els.as_mut().unwrap()
                } else {
                    then
                }
            }
        }
    }
    for tok in tokens {
        match tok {
            TokT::Text(t) => cur_body(&mut stack).push(Node::Text(t)),
            TokT::Var(p) => cur_body(&mut stack).push(Node::Var(p)),
            TokT::Raw(p) => cur_body(&mut stack).push(Node::Raw(p)),
            TokT::Tag(tag) => {
                if let Some(rest) = tag.strip_prefix("for ") {
                    // for <ident> in <path>
                    let mut parts = rest.splitn(2, " in ");
                    let var = parts.next().unwrap_or("");
                    let path_text = parts.next().unwrap_or("");
                    if is_ident(var) && !path_text.is_empty() && !path_text.chars().any(|c| c.is_whitespace()) {
                        let path = path_of(name, path_text)?;
                        stack.push(Frame::For { name: var.to_string(), path, body: Vec::new() });
                    } else {
                        return Err(TemplateError(format!("template \"{}\": {{% {} %}} is not a tag", name, tag)));
                    }
                } else if let Some(rest) = tag.strip_prefix("if ") {
                    let (not, path_text) = if let Some(r2) = rest.strip_prefix("not ") {
                        if r2.is_empty() || r2.chars().any(|c| c.is_whitespace()) {
                            (false, rest)
                        } else {
                            (true, r2)
                        }
                    } else {
                        (false, rest)
                    };
                    if path_text.is_empty() || path_text.chars().any(|c| c.is_whitespace()) {
                        return Err(TemplateError(format!("template \"{}\": {{% {} %}} is not a tag", name, tag)));
                    }
                    let path = path_of(name, path_text)?;
                    stack.push(Frame::If { not, path, then: Vec::new(), els: None, in_else: false });
                } else if tag == "else" {
                    match stack.last_mut().unwrap() {
                        Frame::If { els, in_else, .. } if els.is_none() => {
                            *els = Some(Vec::new());
                            *in_else = true;
                        }
                        _ => return Err(TemplateError(format!("template \"{}\": an else outside an if", name))),
                    }
                } else if tag == "endfor" || tag == "endif" {
                    let want_for = tag == "endfor";
                    let frame = stack.pop().unwrap();
                    match frame {
                        Frame::For { name: n, path, body } if want_for => {
                            cur_body(&mut stack).push(Node::For { name: n, path, body });
                        }
                        Frame::If { not, path, then, els, .. } if !want_for => {
                            cur_body(&mut stack).push(Node::If { not, path, then, els });
                        }
                        _ => {
                            return Err(TemplateError(format!(
                                "template \"{}\": an {} with no open {}",
                                name,
                                tag,
                                &tag[3..]
                            )))
                        }
                    }
                    if stack.is_empty() {
                        return Err(TemplateError(format!("template \"{}\": an {} with no open {}", name, tag, &tag[3..])));
                    }
                } else if let Some(rest) = tag.strip_prefix("include \"") {
                    if let Some(inner) = rest.strip_suffix('"') {
                        if !inner.is_empty() && inner.bytes().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-') {
                            cur_body(&mut stack).push(Node::Include(inner.to_string()));
                            continue;
                        }
                    }
                    return Err(TemplateError(format!("template \"{}\": {{% {} %}} is not a tag", name, tag)));
                } else {
                    return Err(TemplateError(format!("template \"{}\": {{% {} %}} is not a tag", name, tag)));
                }
            }
        }
    }
    if stack.len() > 1 {
        let kind = match stack.last().unwrap() {
            Frame::For { .. } => "for",
            Frame::If { .. } => "if",
            Frame::Root(_) => "root",
        };
        return Err(TemplateError(format!("template \"{}\": a {} is not closed", name, kind)));
    }
    match stack.pop().unwrap() {
        Frame::Root(b) => Ok(b),
        _ => unreachable!(),
    }
}

const INCLUDE_DEPTH: usize = 8;

fn render_nodes(
    nodes: &[Node],
    scope: &mut Scope,
    templates: &HashMap<String, Rc<Vec<Node>>>,
    depth: usize,
    out: &mut String,
) {
    for n in nodes {
        match n {
            Node::Text(t) => out.push_str(t),
            Node::Var(p) => {
                let v = lookup(scope, p);
                escape_html_into(out, &text_of(v.as_ref().unwrap_or(&Val::None)));
            }
            Node::Raw(p) => {
                let v = lookup(scope, p);
                out.push_str(&text_of(v.as_ref().unwrap_or(&Val::None)));
            }
            Node::If { not, path, then, els } => {
                let v = lookup(scope, path);
                let take_then = truthy(v.as_ref()) != *not;
                if take_then {
                    render_nodes(then, scope, templates, depth, out);
                } else if let Some(e) = els {
                    render_nodes(e, scope, templates, depth, out);
                }
            }
            Node::For { name, path, body } => {
                let xs = lookup(scope, path);
                if let Some(Val::Arr(items)) = xs {
                    let len = items.len();
                    for (k, x) in items.iter().enumerate() {
                        // the reference builds { [name]: x, loop: {...} }: a loop variable named
                        // "loop" is overwritten by the loop record, so insert name first, then loop.
                        let mut frame = HashMap::with_capacity(2);
                        frame.insert(name.clone(), x.clone());
                        frame.insert(
                            "loop".to_string(),
                            obj(vec![
                                ("index".to_string(), Val::Num(k as i64 + 1)),
                                ("first".to_string(), Val::Bool(k == 0)),
                                ("last".to_string(), Val::Bool(k == len - 1)),
                                ("length".to_string(), Val::Num(len as i64)),
                            ]),
                        );
                        scope.push(frame);
                        render_nodes(body, scope, templates, depth, out);
                        scope.pop();
                    }
                }
            }
            Node::Include(inc_name) => {
                if depth < INCLUDE_DEPTH {
                    if let Some(t) = templates.get(inc_name) {
                        let t = t.clone();
                        render_nodes(&t, scope, templates, depth + 1, out);
                    }
                }
            }
        }
    }
}

// ---------------------------------------------------------------- the build

struct Source {
    path: String,
    body: String,
    data: Fm,
    post: bool,
    layout: String,
    slug: String,
    out_dir: String,
    url: String,
    html: String,
    toc: Vec<TocItem>,
    excerpt: String,
    newer: Option<usize>,
    older: Option<usize>,
    tag_refs: Vec<(String, String, String)>, // (name, slug, url)
}

struct TagInfo {
    name: String,
    slug: String,
    url: String,
    posts: Vec<usize>,
}

fn dir_of(path: &str) -> String {
    match path.rfind('/') {
        Some(p) => path[..p].to_string(),
        None => String::new(),
    }
}

fn resolve_path(dir: &str, rel: &str) -> Option<String> {
    let mut parts: Vec<&str> = if dir.is_empty() { Vec::new() } else { dir.split('/').collect() };
    for seg in rel.split('/') {
        if seg.is_empty() || seg == "." {
            continue;
        }
        if seg == ".." {
            if parts.is_empty() {
                return None;
            }
            parts.pop();
        } else {
            parts.push(seg);
        }
    }
    Some(parts.join("/"))
}

fn words_of(text: &str) -> usize {
    text.split_whitespace().count()
}

fn link_of(sources: &[Source], s: Option<usize>) -> Val {
    match s {
        None => Val::None,
        Some(i) => {
            let s = &sources[i];
            obj(vec![
                ("title".to_string(), Val::Str(s.data.title.clone().unwrap_or_default())),
                ("url".to_string(), Val::Str(s.url.clone())),
                ("date".to_string(), Val::Str(s.data.date.clone().unwrap_or_default())),
                (
                    "date_long".to_string(),
                    Val::Str(match &s.data.date {
                        None => String::new(),
                        Some(d) => long_date(d),
                    }),
                ),
            ])
        }
    }
}

fn card_of(s: &Source) -> Val {
    obj(vec![
        ("title".to_string(), Val::Str(s.data.title.clone().unwrap_or_default())),
        ("url".to_string(), Val::Str(s.url.clone())),
        ("date".to_string(), Val::Str(s.data.date.clone().unwrap_or_default())),
        (
            "date_long".to_string(),
            Val::Str(match &s.data.date {
                None => String::new(),
                Some(d) => long_date(d),
            }),
        ),
        ("summary".to_string(), Val::Str(s.data.summary.clone().unwrap_or_default())),
        ("excerpt".to_string(), Val::Str(s.excerpt.clone())),
        (
            "reading_minutes".to_string(),
            Val::Num(std::cmp::max(1, (words_of(&s.body) as f64 / 200.0).ceil() as i64)),
        ),
        (
            "tags".to_string(),
            Val::Arr(
                s.tag_refs
                    .iter()
                    .map(|(name, slug, url)| {
                        obj(vec![
                            ("name".to_string(), Val::Str(name.clone())),
                            ("slug".to_string(), Val::Str(slug.clone())),
                            ("url".to_string(), Val::Str(url.clone())),
                        ])
                    })
                    .collect(),
            ),
        ),
    ])
}

fn page_of(sources: &[Source], i: usize) -> Val {
    let s = &sources[i];
    // card fields first, then the page's own (same key order as the reference's spread)
    let card = card_of(s);
    let mut pairs = match card {
        Val::Obj(p) => (*p).clone(),
        _ => unreachable!(),
    };
    pairs.push(("slug".to_string(), Val::Str(s.slug.clone())));
    pairs.push(("content".to_string(), Val::Str(s.html.clone())));
    pairs.push((
        "toc".to_string(),
        Val::Arr(
            s.toc
                .iter()
                .map(|h| {
                    obj(vec![
                        ("level".to_string(), Val::Num(h.level as i64)),
                        ("id".to_string(), Val::Str(h.id.clone())),
                        ("html".to_string(), Val::Str(h.html.clone())),
                    ])
                })
                .collect(),
        ),
    ));
    pairs.push(("newer".to_string(), link_of(sources, s.newer)));
    pairs.push(("older".to_string(), link_of(sources, s.older)));
    pairs.push(("is_post".to_string(), Val::Bool(s.post)));
    Val::Obj(Rc::new(pairs))
}

fn build(input: &serde_json::Value) -> serde_json::Value {
    let empty_obj = serde_json::Value::Object(serde_json::Map::new());
    let site = input.get("site").unwrap_or(&empty_obj);
    let site_title = site.get("title").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let base_raw = site.get("base_url").and_then(|v| v.as_str()).unwrap_or("");
    let base = base_raw.trim_end_matches('/').to_string();
    let per_page = site.get("per_page").and_then(|v| v.as_i64()).unwrap_or(0);
    let recent_n = site.get("recent").and_then(|v| v.as_i64()).unwrap_or(0);

    // templates, parsed in name order; failures go to the report
    let mut report: Vec<String> = Vec::new();
    let mut templates: HashMap<String, Rc<Vec<Node>>> = HashMap::new();
    if let Some(tobj) = input.get("templates").and_then(|v| v.as_object()) {
        let mut names: Vec<&String> = tobj.keys().collect();
        names.sort();
        for name in names {
            let src = tobj[name].as_str().unwrap_or("");
            match parse_template(name, src) {
                Ok(nodes) => {
                    templates.insert(name.clone(), Rc::new(nodes));
                }
                Err(e) => report.push(e.0),
            }
        }
    }

    // sources and statics, in path order
    let mut files: Vec<(String, String)> = Vec::new();
    if let Some(arr) = input.get("files").and_then(|v| v.as_array()) {
        for f in arr {
            files.push((
                f.get("path").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                f.get("content").and_then(|v| v.as_str()).unwrap_or("").to_string(),
            ));
        }
    }
    files.sort_by(|a, b| a.0.cmp(&b.0));
    let mut raw_sources: Vec<(String, String)> = Vec::new();
    let mut statics: Vec<(String, String)> = Vec::new();
    for (path, content) in files {
        if path.ends_with(".md") {
            raw_sources.push((path, content.replace("\r\n", "\n").replace('\r', "\n")));
        } else {
            statics.push((path, content));
        }
    }

    // front matter
    let mut sources: Vec<Source> = Vec::new();
    for (path, text) in raw_sources {
        let (data, body) = match parse_front_matter(&text) {
            Ok(v) => v,
            Err(e) => {
                report.push(format!("refused {}: {}", path, e));
                continue;
            }
        };
        let post = path.starts_with("posts/");
        let mut refused = false;
        for d in ["tags", "page"] {
            if path.starts_with(&format!("{}/", d)) {
                report.push(format!("refused {}: {}/ is the generator's own", path, d));
                refused = true;
                break;
            }
        }
        if refused {
            continue;
        }
        if post && data.date.is_none() {
            report.push(format!("refused {}: a post needs a date", path));
            continue;
        }
        let layout = data
            .layout
            .clone()
            .unwrap_or_else(|| if post { "post".to_string() } else { "page".to_string() });
        if !templates.contains_key(&layout) {
            report.push(format!("refused {}: layout \"{}\" is no template that parses", path, layout));
            continue;
        }
        if data.draft {
            report.push(format!("draft {}", path));
            continue;
        }
        sources.push(Source {
            path,
            body,
            data,
            post,
            layout,
            slug: String::new(),
            out_dir: String::new(),
            url: String::new(),
            html: String::new(),
            toc: Vec::new(),
            excerpt: String::new(),
            newer: None,
            older: None,
            tag_refs: Vec::new(),
        });
    }

    // permalinks
    let mut slug_cache: HashMap<String, String> = HashMap::new();
    let mut taken: HashMap<String, HashSet<String>> = HashMap::new();
    let mut by_source: HashMap<String, usize> = HashMap::new();
    for i in 0..sources.len() {
        let dir = if sources[i].post { "posts".to_string() } else { dir_of(&sources[i].path) };
        let set = taken.entry(dir.clone()).or_insert_with(|| {
            if dir.is_empty() {
                ["posts", "tags", "page"].iter().map(|s| s.to_string()).collect()
            } else {
                HashSet::new()
            }
        });
        let base_slug = match &sources[i].data.slug {
            Some(s) => s.clone(),
            None => {
                let title = sources[i].data.title.clone().unwrap_or_default();
                match slug_cache.get(&title) {
                    Some(v) => v.clone(),
                    None => {
                        let v = slugify(&title);
                        slug_cache.insert(title, v.clone());
                        v
                    }
                }
            }
        };
        let slug = unique_slug(&base_slug, set);
        sources[i].slug = slug.clone();
        sources[i].out_dir = if dir.is_empty() { slug } else { format!("{}/{}", dir, slug) };
        sources[i].url = format!("{}/{}/", base, sources[i].out_dir);
        by_source.insert(sources[i].path.clone(), i);
    }

    // markdown
    let static_paths: HashSet<String> = statics.iter().map(|(p, _)| p.clone()).collect();
    let broken: std::cell::RefCell<HashSet<String>> = std::cell::RefCell::new(HashSet::new());
    for i in 0..sources.len() {
        let dir = dir_of(&sources[i].path);
        let src_path = sources[i].path.clone();
        let resolve = |target: &str| -> String {
            let scheme = {
                let b = target.as_bytes();
                !b.is_empty()
                    && b[0].is_ascii_alphabetic()
                    && {
                        let mut j = 1;
                        while j < b.len() && (b[j].is_ascii_alphanumeric() || matches!(b[j], b'+' | b'-' | b'.')) {
                            j += 1;
                        }
                        j < b.len() && b[j] == b':'
                    }
            };
            if scheme || target.starts_with('/') || target.starts_with('#') {
                return target.to_string();
            }
            let hash = target.find('#');
            let rel = match hash {
                Some(h) => &target[..h],
                None => target,
            };
            let frag = match hash {
                Some(h) => &target[h..],
                None => "",
            };
            let path = if rel.is_empty() { None } else { resolve_path(&dir, rel) };
            if let Some(p) = &path {
                if let Some(idx) = by_source.get(p) {
                    return format!("{}{}", sources[*idx].url, frag);
                }
                if static_paths.contains(p) {
                    return format!("{}/{}{}", base, p, frag);
                }
            }
            broken.borrow_mut().insert(format!("broken-link {} -> {}", src_path, target));
            target.to_string()
        };
        let env = LinkEnv { resolve: &resolve };
        let body = sources[i].body.clone();
        let (html, toc, excerpt) = render_markdown(&body, &env, &mut slug_cache);
        sources[i].html = html;
        sources[i].toc = toc;
        sources[i].excerpt = excerpt;
    }

    // collections
    let mut posts: Vec<usize> = sources
        .iter()
        .enumerate()
        .filter(|(_, s)| s.post)
        .map(|(i, _)| i)
        .collect();
    posts.sort_by(|&x, &y| {
        let dx = sources[x].data.date.as_deref().unwrap_or("");
        let dy = sources[y].data.date.as_deref().unwrap_or("");
        dy.cmp(dx).then_with(|| sources[x].slug.cmp(&sources[y].slug))
    });
    for (k, &p) in posts.iter().enumerate() {
        sources[p].newer = if k == 0 { None } else { Some(posts[k - 1]) };
        sources[p].older = if k == posts.len() - 1 { None } else { Some(posts[k + 1]) };
    }

    // tags
    let own_tags = |sources: &[Source], slug_cache: &mut HashMap<String, String>, i: usize| -> Vec<String> {
        let mut seen: HashSet<String> = HashSet::new();
        let mut out = Vec::new();
        if let Some(tags) = &sources[i].data.tags {
            for name in tags {
                let slug = match slug_cache.get(name) {
                    Some(v) => v.clone(),
                    None => {
                        let v = slugify(name);
                        slug_cache.insert(name.clone(), v.clone());
                        v
                    }
                };
                if seen.insert(slug) {
                    out.push(name.clone());
                }
            }
        }
        out
    };
    let mut tag_list: Vec<TagInfo> = Vec::new();
    let mut tag_by_slug: HashMap<String, usize> = HashMap::new();
    for &p in &posts {
        let names = own_tags(&sources, &mut slug_cache, p);
        let mut refs = Vec::new();
        for name in names {
            let slug = slug_cache.get(&name).unwrap().clone();
            let ti = match tag_by_slug.get(&slug) {
                Some(&ti) => ti,
                None => {
                    let ti = tag_list.len();
                    tag_list.push(TagInfo {
                        name: name.clone(),
                        slug: slug.clone(),
                        url: format!("{}/tags/{}/", base, slug),
                        posts: Vec::new(),
                    });
                    tag_by_slug.insert(slug.clone(), ti);
                    ti
                }
            };
            tag_list[ti].posts.push(p);
            refs.push((tag_list[ti].name.clone(), tag_list[ti].slug.clone(), tag_list[ti].url.clone()));
        }
        sources[p].tag_refs = refs;
    }
    for i in 0..sources.len() {
        if sources[i].post {
            continue;
        }
        let names = own_tags(&sources, &mut slug_cache, i);
        let mut refs = Vec::new();
        for name in names {
            let slug = slug_cache.get(&name).unwrap().clone();
            match tag_by_slug.get(&slug) {
                Some(&ti) => refs.push((tag_list[ti].name.clone(), tag_list[ti].slug.clone(), tag_list[ti].url.clone())),
                None => refs.push((name, slug, String::new())),
            }
        }
        sources[i].tag_refs = refs;
    }
    tag_list.sort_by(|a, b| b.posts.len().cmp(&a.posts.len()).then_with(|| a.slug.cmp(&b.slug)));

    // layouts
    let mut cards: HashMap<usize, Val> = HashMap::new();
    for &p in &posts {
        cards.insert(p, card_of(&sources[p]));
    }
    let site_val = obj(vec![
        ("title".to_string(), Val::Str(site_title)),
        ("base_url".to_string(), Val::Str(base.clone())),
    ]);
    let all_tags = Val::Arr(
        tag_list
            .iter()
            .map(|t| {
                obj(vec![
                    ("name".to_string(), Val::Str(t.name.clone())),
                    ("slug".to_string(), Val::Str(t.slug.clone())),
                    ("url".to_string(), Val::Str(t.url.clone())),
                    ("count".to_string(), Val::Num(t.posts.len() as i64)),
                ])
            })
            .collect(),
    );
    let recent = Val::Arr(
        posts
            .iter()
            .take(if recent_n < 0 { 0 } else { recent_n as usize })
            .map(|p| cards[p].clone())
            .collect(),
    );
    let mut output: serde_json::Map<String, serde_json::Value> = serde_json::Map::new();
    let render = |name: &str,
                  extra: Vec<(String, Val)>,
                  templates: &HashMap<String, Rc<Vec<Node>>>,
                  site_val: &Val,
                  all_tags: &Val,
                  recent: &Val|
     -> Option<String> {
        let nodes = templates.get(name)?;
        let mut root: HashMap<String, Val> = HashMap::new();
        root.insert("site".to_string(), site_val.clone());
        root.insert("tags".to_string(), all_tags.clone());
        root.insert("recent".to_string(), recent.clone());
        for (k, v) in extra {
            root.insert(k, v);
        }
        let mut scope: Scope = vec![root];
        let mut out = String::new();
        render_nodes(nodes, &mut scope, templates, 0, &mut out);
        Some(out)
    };
    for i in 0..sources.len() {
        let page = page_of(&sources, i);
        let html = render(
            &sources[i].layout.clone(),
            vec![("page".to_string(), page)],
            &templates,
            &site_val,
            &all_tags,
            &recent,
        );
        output.insert(
            format!("{}/index.html", sources[i].out_dir),
            serde_json::Value::String(html.unwrap_or_default()),
        );
    }
    let per = per_page;
    let pages = std::cmp::max(1, ((posts.len() as i64) + per - 1) / if per == 0 { 1 } else { per });
    let index_url = |k: i64| -> String {
        if k == 1 {
            format!("{}/", base)
        } else {
            format!("{}/page/{}/", base, k)
        }
    };
    for k in 1..=pages {
        let slice: Vec<Val> = posts
            .iter()
            .skip(((k - 1) * per) as usize)
            .take(if per < 0 { 0 } else { per as usize })
            .map(|p| cards[p].clone())
            .collect();
        let pager = obj(vec![
            ("number".to_string(), Val::Num(k)),
            ("total".to_string(), Val::Num(pages)),
            ("url".to_string(), Val::Str(index_url(k))),
            ("newer_url".to_string(), Val::Str(if k == 1 { String::new() } else { index_url(k - 1) })),
            ("older_url".to_string(), Val::Str(if k == pages { String::new() } else { index_url(k + 1) })),
        ]);
        if let Some(html) = render(
            "index",
            vec![("posts".to_string(), Val::Arr(slice)), ("pager".to_string(), pager)],
            &templates,
            &site_val,
            &all_tags,
            &recent,
        ) {
            output.insert(
                if k == 1 { "index.html".to_string() } else { format!("page/{}/index.html", k) },
                serde_json::Value::String(html),
            );
        }
    }
    for t in &tag_list {
        let tag_val = obj(vec![
            ("name".to_string(), Val::Str(t.name.clone())),
            ("slug".to_string(), Val::Str(t.slug.clone())),
            ("url".to_string(), Val::Str(t.url.clone())),
            ("count".to_string(), Val::Num(t.posts.len() as i64)),
            (
                "posts".to_string(),
                Val::Arr(t.posts.iter().map(|p| cards[p].clone()).collect()),
            ),
        ]);
        if let Some(html) = render("tag", vec![("tag".to_string(), tag_val)], &templates, &site_val, &all_tags, &recent) {
            output.insert(format!("tags/{}/index.html", t.slug), serde_json::Value::String(html));
        }
    }
    if let Some(html) = render("tags", vec![], &templates, &site_val, &all_tags, &recent) {
        output.insert("tags/index.html".to_string(), serde_json::Value::String(html));
    }
    for (path, content) in &statics {
        if output.contains_key(path) {
            report.push(format!("shadowed {}: a built page has its path", path));
        } else {
            output.insert(path.clone(), serde_json::Value::String(content.clone()));
        }
    }

    // report
    let mut lines: Vec<String> = report;
    lines.extend(broken.borrow().iter().cloned());
    lines.sort();
    let page_count = sources.iter().filter(|s| !s.post).count();
    lines.push(format!(
        "built {} pages, {} posts, {} tags and {} static files",
        page_count,
        posts.len(),
        tag_list.len(),
        statics.len()
    ));
    output.insert(
        "build-report.txt".to_string(),
        serde_json::Value::String(format!("{}\n", lines.join("\n"))),
    );
    serde_json::Value::Object(output)
}

/// The harness's whole job in one call: cases.json text in, answers.json text out.
#[napi_derive::napi]
pub fn solve_all(cases_json: String) -> String {
    let parsed: serde_json::Value = match serde_json::from_str(&cases_json) {
        Ok(v) => v,
        Err(_) => return "[]".to_string(),
    };
    let mut answers: Vec<serde_json::Value> = Vec::new();
    if let Some(arr) = parsed.as_array() {
        for c in arr {
            let input = c.get("input").cloned().unwrap_or(serde_json::Value::Null);
            answers.push(build(&input));
        }
    }
    serde_json::to_string(&serde_json::Value::Array(answers)).unwrap_or_else(|_| "[]".to_string())
}

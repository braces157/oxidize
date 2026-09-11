//! IDE-grade syntax highlighting engine for the LazyOx TUI inspector diff view.
//!
//! Provides rich tokenization across Rust, Java, Kotlin, C#, Swift, Python, JavaScript,
//! TypeScript, Go, C/C++, PHP, Ruby, HTML, CSS, SQL, Lua, Zig, Scala, Dart, R, Elixir,
//! Haskell, JSON, TOML, YAML, XML, Dockerfile, GraphQL, Shell, Markdown, and Config files.

use crate::model::{DiffLine, DiffLineKind};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};

/// Programming languages recognized by the syntax highlighter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Language {
    #[default]
    Rust,
    Java,
    Kotlin,
    CSharp,
    Swift,
    Python,
    JavaScript,
    TypeScript,
    Go,
    C,
    Cpp,
    Php,
    Ruby,
    Html,
    Css,
    Sql,
    Lua,
    Zig,
    Scala,
    Dart,
    R,
    Elixir,
    Haskell,
    Json,
    Toml,
    Yaml,
    Xml,
    Shell,
    Markdown,
    Dockerfile,
    Graphql,
    Ini,
    Generic,
}

impl Language {
    /// Detects language from a file path or extension.
    pub fn from_path(path: &str) -> Self {
        let p = path.to_lowercase();
        let filename = p.rsplit(['/', '\\']).next().unwrap_or(&p);

        // Special filenames
        if filename == "dockerfile"
            || filename.ends_with(".dockerfile")
            || filename == "containerfile"
            || filename.ends_with(".containerfile")
        {
            return Language::Dockerfile;
        }
        if filename == "gemfile" || filename == "rakefile" || filename.ends_with(".gemspec") {
            return Language::Ruby;
        }
        if filename == "cargo.lock" {
            return Language::Toml;
        }
        if filename == "makefile" || filename == "gnumakefile" {
            return Language::Shell;
        }
        if filename == ".env"
            || filename.starts_with(".env.")
            || filename == ".editorconfig"
            || filename == ".gitconfig"
        {
            return Language::Ini;
        }

        if p.ends_with(".rs") {
            Language::Rust
        } else if p.ends_with(".java") || p.ends_with(".jar") || p.ends_with(".jsp") {
            Language::Java
        } else if p.ends_with(".kt") || p.ends_with(".kts") {
            Language::Kotlin
        } else if p.ends_with(".cs") || p.ends_with(".csx") {
            Language::CSharp
        } else if p.ends_with(".swift") {
            Language::Swift
        } else if p.ends_with(".php") || p.ends_with(".phtml") {
            Language::Php
        } else if p.ends_with(".rb") || p.ends_with(".rake") {
            Language::Ruby
        } else if p.ends_with(".py") || p.ends_with(".pyw") {
            Language::Python
        } else if p.ends_with(".ts")
            || p.ends_with(".tsx")
            || p.ends_with(".mts")
            || p.ends_with(".cts")
        {
            Language::TypeScript
        } else if p.ends_with(".js")
            || p.ends_with(".jsx")
            || p.ends_with(".mjs")
            || p.ends_with(".cjs")
        {
            Language::JavaScript
        } else if p.ends_with(".go") {
            Language::Go
        } else if p.ends_with(".cpp")
            || p.ends_with(".cc")
            || p.ends_with(".cxx")
            || p.ends_with(".hpp")
            || p.ends_with(".hxx")
            || p.ends_with(".hh")
        {
            Language::Cpp
        } else if p.ends_with(".c") || p.ends_with(".h") {
            Language::C
        } else if p.ends_with(".html")
            || p.ends_with(".htm")
            || p.ends_with(".xhtml")
            || p.ends_with(".svelte")
            || p.ends_with(".vue")
        {
            Language::Html
        } else if p.ends_with(".css")
            || p.ends_with(".scss")
            || p.ends_with(".sass")
            || p.ends_with(".less")
        {
            Language::Css
        } else if p.ends_with(".sql") || p.ends_with(".psql") || p.ends_with(".mysql") {
            Language::Sql
        } else if p.ends_with(".lua") {
            Language::Lua
        } else if p.ends_with(".zig") {
            Language::Zig
        } else if p.ends_with(".scala") || p.ends_with(".sc") {
            Language::Scala
        } else if p.ends_with(".dart") {
            Language::Dart
        } else if p.ends_with(".r") || p.ends_with(".rmd") {
            Language::R
        } else if p.ends_with(".ex") || p.ends_with(".exs") {
            Language::Elixir
        } else if p.ends_with(".hs") || p.ends_with(".lhs") {
            Language::Haskell
        } else if p.ends_with(".graphql") || p.ends_with(".gql") {
            Language::Graphql
        } else if p.ends_with(".json") || p.ends_with(".jsonc") || p.ends_with(".json5") {
            Language::Json
        } else if p.ends_with(".toml") {
            Language::Toml
        } else if p.ends_with(".yaml") || p.ends_with(".yml") {
            Language::Yaml
        } else if p.ends_with(".xml")
            || p.ends_with(".svg")
            || p.ends_with(".xaml")
            || p.ends_with(".plist")
            || p.ends_with(".csproj")
            || p.ends_with(".fsproj")
            || p.ends_with(".vbproj")
            || p.ends_with(".pom")
        {
            Language::Xml
        } else if p.ends_with(".ini")
            || p.ends_with(".cfg")
            || p.ends_with(".conf")
            || p.ends_with(".properties")
        {
            Language::Ini
        } else if p.ends_with(".sh")
            || p.ends_with(".bash")
            || p.ends_with(".zsh")
            || p.ends_with(".fish")
            || p.ends_with(".ps1")
            || p.ends_with(".bat")
            || p.ends_with(".cmd")
        {
            Language::Shell
        } else if p.ends_with(".md") || p.ends_with(".markdown") {
            Language::Markdown
        } else {
            Language::Generic
        }
    }

    /// Detects language from inspector diff view title (e.g. "Diff: src/main.rs (Modified)").
    pub fn from_title(title: &str) -> Self {
        for word in title.split_whitespace() {
            let clean = word
                .trim_matches(|c: char| !c.is_alphanumeric() && c != '.' && c != '_' && c != '-');
            let lang = Self::from_path(clean);
            if lang != Language::Generic {
                return lang;
            }
        }
        Language::Generic
    }

    /// Human-readable language name for inspector badges.
    pub fn name(self) -> &'static str {
        match self {
            Language::Rust => "Rust",
            Language::Java => "Java",
            Language::Kotlin => "Kotlin",
            Language::CSharp => "C#",
            Language::Swift => "Swift",
            Language::Python => "Python",
            Language::JavaScript => "JavaScript",
            Language::TypeScript => "TypeScript",
            Language::Go => "Go",
            Language::C => "C",
            Language::Cpp => "C++",
            Language::Php => "PHP",
            Language::Ruby => "Ruby",
            Language::Html => "HTML",
            Language::Css => "CSS",
            Language::Sql => "SQL",
            Language::Lua => "Lua",
            Language::Zig => "Zig",
            Language::Scala => "Scala",
            Language::Dart => "Dart",
            Language::R => "R",
            Language::Elixir => "Elixir",
            Language::Haskell => "Haskell",
            Language::Json => "JSON",
            Language::Toml => "TOML",
            Language::Yaml => "YAML",
            Language::Xml => "XML",
            Language::Shell => "Shell",
            Language::Markdown => "Markdown",
            Language::Dockerfile => "Dockerfile",
            Language::Graphql => "GraphQL",
            Language::Ini => "Config",
            Language::Generic => "Code",
        }
    }
}

/// State machine tracking active syntax language and rendering highlighted diff lines.
#[derive(Debug, Clone)]
pub struct SyntaxHighlighter {
    /// Active language used for tokenization.
    pub current_lang: Language,
    /// Whether currently inside an unclosed multiline block comment (`/* ... */`).
    pub in_block_comment: bool,
    /// Delimiter if currently inside an unclosed multiline string (e.g. `"` or `'`).
    pub in_multiline_string: Option<char>,
}

impl SyntaxHighlighter {
    /// Creates a new syntax highlighter initialized with a default language.
    pub fn new(initial_lang: Language) -> Self {
        Self {
            current_lang: initial_lang,
            in_block_comment: false,
            in_multiline_string: None,
        }
    }

    /// Highlights a DiffLine with IDE colors and diff status backgrounds.
    pub fn highlight_line(&mut self, dl: &DiffLine) -> Line<'static> {
        // Track language changes dynamically when multi-file diff headers appear
        if dl.content.starts_with("diff --git ") {
            if let Some(path) = parse_diff_git_path(&dl.content) {
                let detected = Language::from_path(&path);
                self.current_lang = detected;
            }
            self.in_block_comment = false;
            self.in_multiline_string = None;
            return highlight_header(&dl.content);
        }

        match dl.kind {
            DiffLineKind::Header => {
                self.in_block_comment = false;
                self.in_multiline_string = None;
                highlight_header(&dl.content)
            }
            DiffLineKind::HunkHeader => {
                self.in_block_comment = false;
                self.in_multiline_string = None;
                highlight_hunk_header(&dl.content)
            }
            DiffLineKind::Addition => {
                let text = dl.content.strip_prefix('+').unwrap_or(&dl.content);
                let bg_color = Color::Rgb(18, 38, 24);
                let mut spans = vec![Span::styled(
                    "+",
                    Style::default()
                        .fg(Color::Rgb(100, 230, 120))
                        .bg(bg_color)
                        .add_modifier(Modifier::BOLD),
                )];
                tokenize_code(
                    text,
                    self.current_lang,
                    Some(bg_color),
                    &mut self.in_block_comment,
                    &mut self.in_multiline_string,
                    &mut spans,
                );
                Line::from(spans)
            }
            DiffLineKind::Deletion => {
                let text = dl.content.strip_prefix('-').unwrap_or(&dl.content);
                let bg_color = Color::Rgb(40, 18, 22);
                let mut spans = vec![Span::styled(
                    "-",
                    Style::default()
                        .fg(Color::Rgb(240, 95, 95))
                        .bg(bg_color)
                        .add_modifier(Modifier::BOLD),
                )];
                tokenize_code(
                    text,
                    self.current_lang,
                    Some(bg_color),
                    &mut self.in_block_comment,
                    &mut self.in_multiline_string,
                    &mut spans,
                );
                Line::from(spans)
            }
            DiffLineKind::Context => {
                let text = dl.content.strip_prefix(' ').unwrap_or(&dl.content);
                let mut spans = vec![Span::styled(" ", Style::default().fg(Color::DarkGray))];
                tokenize_code(
                    text,
                    self.current_lang,
                    None,
                    &mut self.in_block_comment,
                    &mut self.in_multiline_string,
                    &mut spans,
                );
                Line::from(spans)
            }
            DiffLineKind::Normal => {
                let mut spans = Vec::new();
                tokenize_code(
                    &dl.content,
                    self.current_lang,
                    None,
                    &mut self.in_block_comment,
                    &mut self.in_multiline_string,
                    &mut spans,
                );
                Line::from(spans)
            }
        }
    }
}

fn parse_diff_git_path(line: &str) -> Option<String> {
    if let Some(rest) = line.strip_prefix("diff --git ") {
        let trimmed = rest.trim();
        if trimmed.starts_with('"') {
            // Quoted paths: e.g. "a/foo bar.rs" "b/foo bar.rs"
            let mut in_quote = false;
            let mut quotes = Vec::new();
            let mut start = 0;
            for (i, c) in trimmed.char_indices() {
                if c == '"' {
                    if in_quote {
                        quotes.push(&trimmed[start..i]);
                        in_quote = false;
                    } else {
                        in_quote = true;
                        start = i + 1;
                    }
                }
            }
            if let Some(b_part) = quotes.get(1) {
                let clean = b_part.strip_prefix("b/").unwrap_or(b_part);
                return Some(clean.to_string());
            }
        }
        if let Some(idx) = trimmed.rfind(" b/") {
            let b_path = &trimmed[idx + 3..];
            let clean = b_path.trim_matches('"');
            return Some(clean.to_string());
        }
        let parts: Vec<&str> = trimmed.split_whitespace().collect();
        if let Some(b_path) = parts.get(1) {
            let clean = b_path
                .strip_prefix("b/")
                .unwrap_or(b_path)
                .trim_matches('"');
            return Some(clean.to_string());
        }
    }
    None
}

fn highlight_header(content: &str) -> Line<'static> {
    if let Some(sha) = content.strip_prefix("commit ") {
        return Line::from(vec![
            Span::styled(
                "commit ",
                Style::default()
                    .fg(Color::Rgb(198, 120, 221))
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                sha.to_string(),
                Style::default()
                    .fg(Color::Rgb(229, 192, 123))
                    .add_modifier(Modifier::BOLD),
            ),
        ]);
    }

    if let Some(rest) = content.strip_prefix("Author: ") {
        let mut spans = vec![Span::styled(
            "Author: ",
            Style::default()
                .fg(Color::Rgb(97, 175, 239))
                .add_modifier(Modifier::BOLD),
        )];
        if let Some(email_start) = rest.find('<') {
            let name = &rest[..email_start];
            let email = &rest[email_start..];
            spans.push(Span::styled(
                name.to_string(),
                Style::default().fg(Color::Rgb(220, 224, 230)),
            ));
            spans.push(Span::styled(
                email.to_string(),
                Style::default().fg(Color::Rgb(152, 195, 121)),
            ));
        } else {
            spans.push(Span::styled(
                rest.to_string(),
                Style::default().fg(Color::Rgb(220, 224, 230)),
            ));
        }
        return Line::from(spans);
    }

    if let Some(date) = content.strip_prefix("Date:   ") {
        return Line::from(vec![
            Span::styled(
                "Date:   ",
                Style::default()
                    .fg(Color::Rgb(97, 175, 239))
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                date.to_string(),
                Style::default().fg(Color::Rgb(209, 154, 102)),
            ),
        ]);
    }

    if let Some(parents) = content.strip_prefix("Parents: ") {
        return Line::from(vec![
            Span::styled(
                "Parents: ",
                Style::default()
                    .fg(Color::Rgb(97, 175, 239))
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                parents.to_string(),
                Style::default()
                    .fg(Color::Rgb(229, 192, 123))
                    .add_modifier(Modifier::BOLD),
            ),
        ]);
    }

    if let Some(paths) = content.strip_prefix("diff --git ") {
        return Line::from(vec![
            Span::styled(
                "diff --git ",
                Style::default()
                    .fg(Color::Rgb(198, 120, 221))
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                paths.to_string(),
                Style::default()
                    .fg(Color::Rgb(86, 182, 194))
                    .add_modifier(Modifier::BOLD),
            ),
        ]);
    }

    if let Some(file) = content.strip_prefix("--- ") {
        return Line::from(vec![
            Span::styled(
                "--- ",
                Style::default()
                    .fg(Color::Rgb(240, 95, 95))
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                file.to_string(),
                Style::default().fg(Color::Rgb(240, 95, 95)),
            ),
        ]);
    }

    if let Some(file) = content.strip_prefix("+++ ") {
        return Line::from(vec![
            Span::styled(
                "+++ ",
                Style::default()
                    .fg(Color::Rgb(100, 230, 120))
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                file.to_string(),
                Style::default().fg(Color::Rgb(100, 230, 120)),
            ),
        ]);
    }

    // Default header styling
    Line::from(Span::styled(
        content.to_string(),
        Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD),
    ))
}

fn highlight_hunk_header(content: &str) -> Line<'static> {
    if let Some(rest) = content.strip_prefix("@@") {
        if let Some(inner_end) = rest.find("@@") {
            let end_idx = inner_end + 4;
            let range_part = &content[..end_idx];
            let banner = &content[end_idx..];
            return Line::from(vec![
                Span::styled(
                    range_part.to_string(),
                    Style::default()
                        .fg(Color::Rgb(86, 182, 194))
                        .bg(Color::Rgb(20, 32, 45))
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    banner.to_string(),
                    Style::default()
                        .fg(Color::Rgb(140, 165, 185))
                        .bg(Color::Rgb(20, 32, 45)),
                ),
            ]);
        }
    }

    Line::from(Span::styled(
        content.to_string(),
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD),
    ))
}

fn tokenize_code(
    text: &str,
    lang: Language,
    bg: Option<Color>,
    in_block_comment: &mut bool,
    in_multiline_string: &mut Option<char>,
    spans: &mut Vec<Span<'static>>,
) {
    let make_style = |fg: Color, modifier: Modifier| {
        let mut s = Style::default().fg(fg);
        if let Some(b) = bg {
            s = s.bg(b);
        }
        if !modifier.is_empty() {
            s = s.add_modifier(modifier);
        }
        s
    };

    let style_plain = make_style(Color::Rgb(220, 224, 230), Modifier::empty());
    let style_kw_control = make_style(Color::Rgb(224, 108, 117), Modifier::BOLD);
    let style_kw_def = make_style(Color::Rgb(198, 120, 221), Modifier::BOLD);
    let style_type = make_style(Color::Rgb(229, 192, 123), Modifier::empty());
    let style_fn = make_style(Color::Rgb(97, 175, 239), Modifier::empty());
    let style_string = make_style(Color::Rgb(152, 195, 121), Modifier::empty());
    let style_number = make_style(Color::Rgb(209, 154, 102), Modifier::empty());
    let style_const = make_style(Color::Rgb(209, 154, 102), Modifier::BOLD);
    let style_comment = make_style(Color::Rgb(110, 118, 130), Modifier::ITALIC);
    let style_doc_comment = make_style(Color::Rgb(110, 155, 170), Modifier::ITALIC);
    let style_attr = make_style(Color::Rgb(86, 182, 194), Modifier::empty());
    let style_op = make_style(Color::Rgb(86, 182, 194), Modifier::empty());
    let style_punct = make_style(Color::Rgb(171, 178, 191), Modifier::empty());

    let chars: Vec<char> = text.chars().collect();
    let len = chars.len();
    let mut i = 0;

    // Continue multiline block comment if active
    if *in_block_comment {
        let mut closed = false;
        let mut close_idx = 0;
        let mut j = 0;
        while j + 1 < len {
            if chars[j] == '*' && chars[j + 1] == '/' {
                closed = true;
                close_idx = j + 2;
                break;
            }
            j += 1;
        }
        if closed {
            let comment: String = chars[..close_idx].iter().collect();
            spans.push(Span::styled(comment, style_comment));
            *in_block_comment = false;
            i = close_idx;
        } else {
            spans.push(Span::styled(text.to_string(), style_comment));
            return;
        }
    }

    // Continue multiline string if active
    if let Some(q) = *in_multiline_string {
        let mut closed = false;
        let mut close_idx = 0;
        let mut j = 0;
        while j + 2 < len {
            if chars[j] == q && chars[j + 1] == q && chars[j + 2] == q {
                closed = true;
                close_idx = j + 3;
                break;
            }
            j += 1;
        }
        if closed {
            let s: String = chars[..close_idx].iter().collect();
            spans.push(Span::styled(s, style_string));
            *in_multiline_string = None;
            i = close_idx;
        } else {
            spans.push(Span::styled(text.to_string(), style_string));
            return;
        }
    }

    while i < len {
        let loop_start = i;
        let c = chars[i];

        // 1. Whitespace
        if c.is_whitespace() {
            let start = i;
            while i < len && chars[i].is_whitespace() {
                i += 1;
            }
            let ws: String = chars[start..i].iter().collect();
            spans.push(Span::styled(ws, style_plain));
            if i <= loop_start {
                i = loop_start + 1;
            }
            continue;
        }

        // 2. Block Comments: /* ... */
        if c == '/' && i + 1 < len && chars[i + 1] == '*' {
            let start = i;
            i += 2;
            let is_doc = i < len && chars[i] == '*';
            let mut closed = false;
            while i + 1 < len {
                if chars[i] == '*' && chars[i + 1] == '/' {
                    i += 2;
                    closed = true;
                    break;
                }
                i += 1;
            }
            if !closed {
                i = len;
                *in_block_comment = true;
            }
            let comment: String = chars[start..i].iter().collect();
            spans.push(Span::styled(
                comment,
                if is_doc {
                    style_doc_comment
                } else {
                    style_comment
                },
            ));
            if i <= loop_start {
                i = loop_start + 1;
            }
            continue;
        }

        // 3. Line Comments: //
        if c == '/' && i + 1 < len && chars[i + 1] == '/' {
            let comment_text: String = chars[i..].iter().collect();
            let is_doc = comment_text.starts_with("///") || comment_text.starts_with("//!");
            spans.push(Span::styled(
                comment_text,
                if is_doc {
                    style_doc_comment
                } else {
                    style_comment
                },
            ));
            break;
        }

        // 4. Hash Comments: # (Python, Shell, YAML, TOML, Ruby, R, Dockerfile, Ini, PHP)
        if c == '#'
            && matches!(
                lang,
                Language::Python
                    | Language::Shell
                    | Language::Yaml
                    | Language::Toml
                    | Language::Ruby
                    | Language::R
                    | Language::Dockerfile
                    | Language::Ini
                    | Language::Php
            )
        {
            let comment_text: String = chars[i..].iter().collect();
            spans.push(Span::styled(comment_text, style_comment));
            break;
        }

        // 5. Dash Comments: -- (SQL, Lua, Haskell)
        if c == '-'
            && i + 1 < len
            && chars[i + 1] == '-'
            && matches!(lang, Language::Sql | Language::Lua | Language::Haskell)
        {
            let comment_text: String = chars[i..].iter().collect();
            spans.push(Span::styled(comment_text, style_comment));
            break;
        }

        // 6. HTML / XML Comments: <!-- ... -->
        if c == '<'
            && i + 3 < len
            && chars[i + 1] == '!'
            && chars[i + 2] == '-'
            && chars[i + 3] == '-'
            && matches!(lang, Language::Html | Language::Xml)
        {
            let start = i;
            i += 4;
            while i + 2 < len && !(chars[i] == '-' && chars[i + 1] == '-' && chars[i + 2] == '>') {
                i += 1;
            }
            if i + 2 < len {
                i += 3;
            } else {
                i = len;
            }
            let comment: String = chars[start..i].iter().collect();
            spans.push(Span::styled(comment, style_comment));
            continue;
        }

        // 7. Semicolon Comments: ; (Ini)
        if c == ';' && matches!(lang, Language::Ini) {
            let comment_text: String = chars[i..].iter().collect();
            spans.push(Span::styled(comment_text, style_comment));
            break;
        }

        // 8. Rust Attributes / Macros: #[...] or #![...]
        if c == '#'
            && matches!(lang, Language::Rust)
            && i + 1 < len
            && (chars[i + 1] == '[' || chars[i + 1] == '!')
        {
            let start = i;
            while i < len && chars[i] != ']' {
                i += 1;
            }
            if i < len {
                i += 1;
            }
            let attr: String = chars[start..i].iter().collect();
            spans.push(Span::styled(attr, style_attr));
            continue;
        }

        // 9. Annotations & Decorators: @Identifier (Java, Kotlin, Python, TS, C#, Dart, etc.)
        if c == '@' && i + 1 < len && (chars[i + 1].is_alphabetic() || chars[i + 1] == '_') {
            let start = i;
            i += 1;
            while i < len && (chars[i].is_alphanumeric() || chars[i] == '_' || chars[i] == '.') {
                i += 1;
            }
            let attr: String = chars[start..i].iter().collect();
            spans.push(Span::styled(attr, style_attr));
            continue;
        }

        // 10. Preprocessor Directives: #include, #define, #pragma (C, C++, C#)
        if c == '#' && matches!(lang, Language::C | Language::Cpp | Language::CSharp) {
            let mut j = i + 1;
            while j < len && chars[j].is_whitespace() {
                j += 1;
            }
            if j < len && chars[j].is_alphabetic() {
                let start = i;
                while j < len && (chars[j].is_alphanumeric() || chars[j] == '_') {
                    j += 1;
                }
                i = j;
                let dir: String = chars[start..i].iter().collect();
                spans.push(Span::styled(dir, style_kw_control));
                continue;
            }
        }

        // 11. CSS Hex Colors: #fff, #123456
        if c == '#'
            && matches!(lang, Language::Css)
            && i + 1 < len
            && chars[i + 1].is_ascii_hexdigit()
        {
            let start = i;
            i += 1;
            while i < len && chars[i].is_ascii_hexdigit() {
                i += 1;
            }
            let hex: String = chars[start..i].iter().collect();
            spans.push(Span::styled(hex, style_number));
            continue;
        }

        // 12. HTML / XML Tag Names: <tag or </tag
        if c == '<' && matches!(lang, Language::Html | Language::Xml) {
            if i + 1 < len && chars[i + 1] == '/' {
                let start = i;
                i += 2;
                while i < len
                    && (chars[i].is_alphanumeric()
                        || chars[i] == '-'
                        || chars[i] == '_'
                        || chars[i] == ':')
                {
                    i += 1;
                }
                let tag: String = chars[start..i].iter().collect();
                spans.push(Span::styled(tag, style_kw_control));
                continue;
            } else if i + 1 < len && (chars[i + 1].is_alphabetic() || chars[i + 1] == '!') {
                let start = i;
                i += 1;
                while i < len
                    && (chars[i].is_alphanumeric()
                        || chars[i] == '-'
                        || chars[i] == '_'
                        || chars[i] == ':')
                {
                    i += 1;
                }
                let tag: String = chars[start..i].iter().collect();
                spans.push(Span::styled(tag, style_kw_control));
                continue;
            }
        }

        // 13. Triple-Quoted Strings: """...""" or '''...'''
        if (c == '"' || c == '\'') && i + 2 < len && chars[i + 1] == c && chars[i + 2] == c {
            let quote = c;
            let start = i;
            i += 3;
            let mut closed = false;
            while i + 2 < len {
                if chars[i] == quote && chars[i + 1] == quote && chars[i + 2] == quote {
                    i += 3;
                    closed = true;
                    break;
                }
                i += 1;
            }
            if !closed {
                i = len;
                *in_multiline_string = Some(quote);
            }
            let s: String = chars[start..i].iter().collect();
            spans.push(Span::styled(s, style_string));
            if i <= loop_start {
                i = loop_start + 1;
            }
            continue;
        }

        // 14. Standard Strings: "...", '...', `...`
        if c == '"' || c == '\'' || c == '`' {
            let quote = c;
            let start = i;
            i += 1;
            let mut escaped = false;
            while i < len {
                let ch = chars[i];
                if escaped {
                    escaped = false;
                } else if ch == '\\' {
                    escaped = true;
                } else if ch == quote {
                    i += 1;
                    break;
                }
                i += 1;
            }
            let s: String = chars[start..i].iter().collect();
            spans.push(Span::styled(s, style_string));
            if i <= loop_start {
                i = loop_start + 1;
            }
            continue;
        }

        // 15. Numbers & Hex/Units
        if c.is_ascii_digit() || (c == '.' && i + 1 < len && chars[i + 1].is_ascii_digit()) {
            let start = i;
            while i < len
                && (chars[i].is_ascii_alphanumeric()
                    || chars[i] == '.'
                    || chars[i] == '_'
                    || (matches!(lang, Language::Css) && chars[i] == '%'))
            {
                i += 1;
            }
            let num: String = chars[start..i].iter().collect();
            spans.push(Span::styled(num, style_number));
            if i <= loop_start {
                i = loop_start + 1;
            }
            continue;
        }

        // 16. Words / Identifiers & Variables
        if c.is_alphabetic() || c == '_' || c == '$' {
            let start = i;
            if chars[i] == '$' {
                i += 1;
            }
            while i < len && (chars[i].is_alphanumeric() || chars[i] == '_' || chars[i] == '$') {
                i += 1;
            }
            let word: String = chars[start..i].iter().collect();

            // Look ahead to check if followed by `(` (function call)
            let mut next_idx = i;
            while next_idx < len && chars[next_idx].is_whitespace() {
                next_idx += 1;
            }
            let is_fn_call = next_idx < len && chars[next_idx] == '(';

            let style = if is_control_keyword(&word, lang) {
                style_kw_control
            } else if is_def_keyword(&word, lang) {
                style_kw_def
            } else if is_boolean_or_const(&word, lang) {
                style_const
            } else if is_type_name(&word, lang) {
                style_type
            } else if is_fn_call {
                style_fn
            } else {
                style_plain
            };

            spans.push(Span::styled(word, style));
            if i <= loop_start {
                i = loop_start + 1;
            }
            continue;
        }

        // 17. Multi-character and single-character operators
        if is_operator_char(c) {
            let start = i;
            while i < len && is_operator_char(chars[i]) {
                i += 1;
            }
            let op: String = chars[start..i].iter().collect();
            spans.push(Span::styled(op, style_op));
            if i <= loop_start {
                i = loop_start + 1;
            }
            continue;
        }

        // 18. Punctuation / Delimiters: (, ), [, ], {, }, ;, ,, .
        spans.push(Span::styled(c.to_string(), style_punct));
        i += 1;
        if i <= loop_start {
            i = loop_start + 1;
        }
    }
}

fn is_operator_char(c: char) -> bool {
    matches!(
        c,
        '+' | '-' | '*' | '/' | '%' | '=' | '<' | '>' | '!' | '&' | '|' | '^' | '~' | '?' | ':'
    )
}

fn is_control_keyword(w: &str, lang: Language) -> bool {
    let lower = w.to_ascii_lowercase();

    if lang == Language::Sql {
        return matches!(
            lower.as_str(),
            "select"
                | "from"
                | "where"
                | "insert"
                | "into"
                | "values"
                | "update"
                | "set"
                | "delete"
                | "create"
                | "table"
                | "view"
                | "index"
                | "alter"
                | "drop"
                | "truncate"
                | "join"
                | "inner"
                | "left"
                | "right"
                | "full"
                | "outer"
                | "cross"
                | "on"
                | "group"
                | "by"
                | "order"
                | "having"
                | "limit"
                | "offset"
                | "union"
                | "all"
                | "distinct"
                | "as"
                | "and"
                | "or"
                | "not"
                | "in"
                | "is"
                | "like"
                | "ilike"
                | "between"
                | "exists"
                | "case"
                | "when"
                | "then"
                | "else"
                | "end"
                | "cast"
                | "with"
        );
    }

    if lang == Language::Dockerfile {
        return matches!(
            lower.as_str(),
            "from"
                | "run"
                | "cmd"
                | "label"
                | "expose"
                | "env"
                | "add"
                | "copy"
                | "entrypoint"
                | "volume"
                | "user"
                | "workdir"
                | "arg"
                | "onbuild"
                | "stopsignal"
                | "healthcheck"
                | "shell"
                | "as"
        );
    }

    if lang == Language::Graphql {
        return matches!(
            w,
            "query" | "mutation" | "subscription" | "fragment" | "on" | "directive"
        );
    }

    matches!(
        w,
        "if" | "else"
            | "elif"
            | "elsif"
            | "match"
            | "switch"
            | "case"
            | "default"
            | "while"
            | "for"
            | "foreach"
            | "do"
            | "loop"
            | "in"
            | "break"
            | "continue"
            | "return"
            | "yield"
            | "await"
            | "try"
            | "catch"
            | "finally"
            | "throw"
            | "throws"
            | "raise"
            | "except"
            | "rescue"
            | "ensure"
            | "unless"
            | "until"
            | "select"
            | "defer"
            | "goto"
            | "fallthrough"
            | "guard"
            | "repeat"
            | "when"
            | "lock"
            | "assert"
    )
}

fn is_def_keyword(w: &str, lang: Language) -> bool {
    let lower = w.to_ascii_lowercase();

    if lang == Language::Sql {
        return matches!(
            lower.as_str(),
            "primary"
                | "key"
                | "foreign"
                | "references"
                | "constraint"
                | "default"
                | "check"
                | "unique"
                | "auto_increment"
                | "grant"
                | "revoke"
                | "transaction"
                | "commit"
                | "rollback"
                | "procedure"
                | "function"
                | "database"
                | "schema"
                | "trigger"
        );
    }

    if lang == Language::Graphql {
        return matches!(
            w,
            "type"
                | "interface"
                | "union"
                | "schema"
                | "enum"
                | "input"
                | "implements"
                | "extend"
                | "scalar"
        );
    }

    matches!(
        w,
        // Common definitions & systems
        "fn" | "def"
            | "func"
            | "function"
            | "fun"
            | "class"
            | "struct"
            | "enum"
            | "trait"
            | "impl"
            | "interface"
            | "record"
            | "type"
            | "let"
            | "mut"
            | "const"
            | "var"
            | "val"
            | "pub"
            | "mod"
            | "use"
            | "import"
            | "export"
            | "from"
            | "package"
            | "crate"
            | "extern"
            | "static"
            | "async"
            | "unsafe"
            | "where"
            | "ref"
            | "as"
            | "dyn"
            | "super"
            | "new"
            | "delete"
            | "extends"
            | "implements"
            | "lambda"
            | "with"
            | "pass"
            | "global"
            | "nonlocal"
            | "comptime"
            | "inline"
        // Java / Kotlin / C# / Swift / PHP / Scala / Dart keywords
            | "public"
            | "private"
            | "protected"
            | "internal"
            | "final"
            | "abstract"
            | "native"
            | "synchronized"
            | "transient"
            | "volatile"
            | "strictfp"
            | "override"
            | "virtual"
            | "sealed"
            | "readonly"
            | "fixed"
            | "stackalloc"
            | "operator"
            | "implicit"
            | "explicit"
            | "namespace"
            | "using"
            | "delegate"
            | "event"
            | "open"
            | "data"
            | "suspend"
            | "companion"
            | "init"
            | "constructor"
            | "lateinit"
            | "protocol"
            | "extension"
            | "mutating"
            | "nonmutating"
            | "actor"
            | "convenience"
            | "subscript"
            | "deinit"
            | "associatedtype"
            | "module"
            | "defp"
            | "alias"
            | "require"
            | "echo"
            | "print"
            | "include"
            | "require_once"
            | "include_once"
            | "self"
            | "this"
            | "base"
            | "it"
            | "get"
            | "set"
    )
}

fn is_boolean_or_const(w: &str, lang: Language) -> bool {
    let lower = w.to_ascii_lowercase();
    if lang == Language::Sql && (lower == "null" || lower == "true" || lower == "false") {
        return true;
    }
    matches!(
        w,
        "true"
            | "false"
            | "True"
            | "False"
            | "TRUE"
            | "FALSE"
            | "None"
            | "null"
            | "Null"
            | "NULL"
            | "nil"
            | "Nil"
            | "NIL"
            | "undefined"
            | "NaN"
            | "Some"
            | "Ok"
            | "Err"
            | "iota"
    )
}

fn is_type_name(w: &str, lang: Language) -> bool {
    let lower = w.to_ascii_lowercase();

    if lang == Language::Sql {
        return matches!(
            lower.as_str(),
            "int"
                | "integer"
                | "bigint"
                | "smallint"
                | "tinyint"
                | "decimal"
                | "numeric"
                | "float"
                | "real"
                | "double"
                | "varchar"
                | "char"
                | "text"
                | "blob"
                | "clob"
                | "boolean"
                | "bool"
                | "date"
                | "time"
                | "datetime"
                | "timestamp"
                | "json"
                | "jsonb"
                | "uuid"
                | "serial"
                | "bigserial"
                | "bytea"
        );
    }

    matches!(
        w,
        // Primitive & common types
        "bool"
            | "boolean"
            | "char"
            | "str"
            | "string"
            | "String"
            | "Option"
            | "Result"
            | "Vec"
            | "Box"
            | "Rc"
            | "Arc"
            | "Cell"
            | "RefCell"
            | "HashMap"
            | "BTreeMap"
            | "HashSet"
            | "BTreeSet"
            | "i8"
            | "i16"
            | "i32"
            | "i64"
            | "i128"
            | "isize"
            | "u8"
            | "u16"
            | "u32"
            | "u64"
            | "u128"
            | "usize"
            | "f32"
            | "f64"
            | "int"
            | "float"
            | "double"
            | "long"
            | "short"
            | "byte"
            | "void"
            | "sbyte"
            | "ushort"
            | "uint"
            | "ulong"
            | "decimal"
            | "nint"
            | "nuint"
            | "dynamic"
            | "list"
            | "dict"
            | "set"
            | "tuple"
            | "bytes"
            | "Self"
            | "any"
            | "unknown"
            | "never"
            | "number"
            | "object"
            | "symbol"
            | "bigint"
            | "Array"
            | "Object"
            | "Promise"
            | "Error"
            | "Exception"
            | "uint8"
            | "uint16"
            | "uint32"
            | "uint64"
            | "uintptr"
            | "rune"
            | "float32"
            | "float64"
            | "error"
        // Java / Kotlin / C# / Swift standard types
            | "Int"
            | "Long"
            | "Short"
            | "Byte"
            | "Float"
            | "Double"
            | "Boolean"
            | "Char"
            | "Unit"
            | "Any"
            | "Nothing"
            | "Integer"
            | "Character"
            | "List"
            | "Map"
            | "Set"
            | "Collection"
            | "Optional"
            | "Future"
            | "CompletableFuture"
            | "Stream"
            | "ArrayList"
            | "StringBuilder"
            | "StringBuffer"
            | "DateTime"
            | "TimeSpan"
            | "Task"
            | "ValueTask"
            | "Action"
            | "Func"
            | "Guid"
            | "Span"
            | "Memory"
    ) || (w.len() > 1 && w.chars().next().is_some_and(|c| c.is_ascii_uppercase()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_language_detection_from_path() {
        assert_eq!(Language::from_path("src/main.rs"), Language::Rust);
        assert_eq!(Language::from_path("src/App.java"), Language::Java);
        assert_eq!(Language::from_path("src/Model.kt"), Language::Kotlin);
        assert_eq!(Language::from_path("Program.cs"), Language::CSharp);
        assert_eq!(Language::from_path("iOS/View.swift"), Language::Swift);
        assert_eq!(Language::from_path("app/server.py"), Language::Python);
        assert_eq!(Language::from_path("web/index.ts"), Language::TypeScript);
        assert_eq!(Language::from_path("web/App.tsx"), Language::TypeScript);
        assert_eq!(Language::from_path("web/bundle.js"), Language::JavaScript);
        assert_eq!(Language::from_path("main.go"), Language::Go);
        assert_eq!(Language::from_path("engine.cpp"), Language::Cpp);
        assert_eq!(Language::from_path("util.c"), Language::C);
        assert_eq!(Language::from_path("index.php"), Language::Php);
        assert_eq!(Language::from_path("app.rb"), Language::Ruby);
        assert_eq!(Language::from_path("index.html"), Language::Html);
        assert_eq!(Language::from_path("style.css"), Language::Css);
        assert_eq!(Language::from_path("schema.sql"), Language::Sql);
        assert_eq!(Language::from_path("init.lua"), Language::Lua);
        assert_eq!(Language::from_path("main.zig"), Language::Zig);
        assert_eq!(Language::from_path("Spark.scala"), Language::Scala);
        assert_eq!(Language::from_path("flutter.dart"), Language::Dart);
        assert_eq!(Language::from_path("analysis.R"), Language::R);
        assert_eq!(Language::from_path("mix.ex"), Language::Elixir);
        assert_eq!(Language::from_path("logic.hs"), Language::Haskell);
        assert_eq!(Language::from_path("data.json"), Language::Json);
        assert_eq!(Language::from_path("Cargo.toml"), Language::Toml);
        assert_eq!(Language::from_path("deploy.yaml"), Language::Yaml);
        assert_eq!(Language::from_path("pom.xml"), Language::Xml);
        assert_eq!(Language::from_path("run.sh"), Language::Shell);
        assert_eq!(Language::from_path("README.md"), Language::Markdown);
        assert_eq!(Language::from_path("Dockerfile"), Language::Dockerfile);
        assert_eq!(Language::from_path("api.graphql"), Language::Graphql);
        assert_eq!(Language::from_path(".env"), Language::Ini);
        assert_eq!(Language::from_path("LICENSE"), Language::Generic);
    }

    #[test]
    fn test_language_detection_from_title() {
        assert_eq!(
            Language::from_title("Diff: crates/tui/src/main.rs (Modified)"),
            Language::Rust
        );
        assert_eq!(
            Language::from_title("Diff: src/main/java/com/App.java (Modified)"),
            Language::Java
        );
        assert_eq!(
            Language::from_title("Diff: frontend/component.tsx"),
            Language::TypeScript
        );
        assert_eq!(
            Language::from_title("Diff: script.py (Staged)"),
            Language::Python
        );
        assert_eq!(
            Language::from_title("Diff: db/migrations/001_init.sql"),
            Language::Sql
        );
        assert_eq!(Language::from_title("Commit: abcd123"), Language::Generic);
    }

    #[test]
    fn test_highlight_headers_and_hunks() {
        let mut highlighter = SyntaxHighlighter::new(Language::Generic);

        let dl_commit = DiffLine {
            kind: DiffLineKind::Header,
            content: "commit a1b2c3d4e5f6".to_string(),
        };
        let line_commit = highlighter.highlight_line(&dl_commit);
        assert_eq!(line_commit.spans.len(), 2);
        assert_eq!(line_commit.spans[0].content, "commit ");

        let dl_author = DiffLine {
            kind: DiffLineKind::Header,
            content: "Author: Alice <alice@example.com>".to_string(),
        };
        let line_author = highlighter.highlight_line(&dl_author);
        assert_eq!(line_author.spans.len(), 3);
        assert_eq!(line_author.spans[0].content, "Author: ");
        assert_eq!(line_author.spans[1].content, "Alice ");
        assert_eq!(line_author.spans[2].content, "<alice@example.com>");

        let dl_hunk = DiffLine {
            kind: DiffLineKind::HunkHeader,
            content: "@@ -1,5 +1,6 @@ fn example()".to_string(),
        };
        let line_hunk = highlighter.highlight_line(&dl_hunk);
        assert_eq!(line_hunk.spans.len(), 2);
        assert_eq!(line_hunk.spans[0].content, "@@ -1,5 +1,6 @@");
        assert_eq!(line_hunk.spans[1].content, " fn example()");
    }

    #[test]
    fn test_highlight_addition_and_deletion() {
        let mut highlighter = SyntaxHighlighter::new(Language::Rust);

        let dl_add = DiffLine {
            kind: DiffLineKind::Addition,
            content: "+    let answer: i32 = 42;".to_string(),
        };
        let line_add = highlighter.highlight_line(&dl_add);
        assert_eq!(line_add.spans[0].content, "+");
        assert_eq!(line_add.spans[0].style.bg, Some(Color::Rgb(18, 38, 24)));

        let spans_text: String = line_add.spans.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(spans_text, "+    let answer: i32 = 42;");

        // Verify keywords and numbers got styled
        let let_span = line_add.spans.iter().find(|s| s.content == "let").unwrap();
        assert_eq!(let_span.style.fg, Some(Color::Rgb(198, 120, 221)));

        let num_span = line_add.spans.iter().find(|s| s.content == "42").unwrap();
        assert_eq!(num_span.style.fg, Some(Color::Rgb(209, 154, 102)));

        let type_span = line_add.spans.iter().find(|s| s.content == "i32").unwrap();
        assert_eq!(type_span.style.fg, Some(Color::Rgb(229, 192, 123)));

        // Deletion
        let dl_del = DiffLine {
            kind: DiffLineKind::Deletion,
            content: "-    println!(\"hello\");".to_string(),
        };
        let line_del = highlighter.highlight_line(&dl_del);
        assert_eq!(line_del.spans[0].content, "-");
        assert_eq!(line_del.spans[0].style.bg, Some(Color::Rgb(40, 18, 22)));
        let str_span = line_del
            .spans
            .iter()
            .find(|s| s.content == "\"hello\"")
            .unwrap();
        assert_eq!(str_span.style.fg, Some(Color::Rgb(152, 195, 121)));
    }

    #[test]
    fn test_dynamic_language_switch_in_multi_file_diff() {
        let mut highlighter = SyntaxHighlighter::new(Language::Generic);

        let dl_diff_py = DiffLine {
            kind: DiffLineKind::Header,
            content: "diff --git a/server.py b/server.py".to_string(),
        };
        highlighter.highlight_line(&dl_diff_py);
        assert_eq!(highlighter.current_lang, Language::Python);

        let dl_diff_java = DiffLine {
            kind: DiffLineKind::Header,
            content: "diff --git a/src/App.java b/src/App.java".to_string(),
        };
        highlighter.highlight_line(&dl_diff_java);
        assert_eq!(highlighter.current_lang, Language::Java);
    }

    #[test]
    fn test_java_kotlin_csharp_highlighting() {
        let mut highlighter = SyntaxHighlighter::new(Language::Java);
        let dl = DiffLine {
            kind: DiffLineKind::Normal,
            content: "    @Override public String getStatus() { return null; }".to_string(),
        };
        let line = highlighter.highlight_line(&dl);
        let override_span = line
            .spans
            .iter()
            .find(|s| s.content == "@Override")
            .unwrap();
        assert_eq!(override_span.style.fg, Some(Color::Rgb(86, 182, 194))); // style_attr

        let pub_span = line.spans.iter().find(|s| s.content == "public").unwrap();
        assert_eq!(pub_span.style.fg, Some(Color::Rgb(198, 120, 221))); // style_kw_def

        let str_span = line.spans.iter().find(|s| s.content == "String").unwrap();
        assert_eq!(str_span.style.fg, Some(Color::Rgb(229, 192, 123))); // style_type

        let ret_span = line.spans.iter().find(|s| s.content == "return").unwrap();
        assert_eq!(ret_span.style.fg, Some(Color::Rgb(224, 108, 117))); // style_kw_control

        let null_span = line.spans.iter().find(|s| s.content == "null").unwrap();
        assert_eq!(null_span.style.fg, Some(Color::Rgb(209, 154, 102))); // style_const
    }

    #[test]
    fn test_sql_and_dockerfile_highlighting() {
        let mut highlighter = SyntaxHighlighter::new(Language::Sql);
        let dl_sql = DiffLine {
            kind: DiffLineKind::Normal,
            content: "SELECT id, name FROM users WHERE active = true; -- check users".to_string(),
        };
        let line_sql = highlighter.highlight_line(&dl_sql);
        let select_span = line_sql
            .spans
            .iter()
            .find(|s| s.content == "SELECT")
            .unwrap();
        assert_eq!(select_span.style.fg, Some(Color::Rgb(224, 108, 117)));

        let comment_span = line_sql
            .spans
            .iter()
            .find(|s| s.content.contains("-- check users"))
            .unwrap();
        assert_eq!(comment_span.style.fg, Some(Color::Rgb(110, 118, 130)));

        let mut docker_highlighter = SyntaxHighlighter::new(Language::Dockerfile);
        let dl_docker = DiffLine {
            kind: DiffLineKind::Normal,
            content: "FROM alpine:latest".to_string(),
        };
        let line_docker = docker_highlighter.highlight_line(&dl_docker);
        let from_span = line_docker
            .spans
            .iter()
            .find(|s| s.content == "FROM")
            .unwrap();
        assert_eq!(from_span.style.fg, Some(Color::Rgb(224, 108, 117)));
    }

    #[test]
    fn test_highlighter_termination_and_dollar_tokens() {
        let test_cases = [
            (Language::Shell, "echo $HOME and $$ and $? and $1"),
            (Language::JavaScript, "const $value = $el.find('#id');"),
            (Language::Php, "echo $variable . ' = ' . $_POST['val'];"),
            (
                Language::Graphql,
                "query GetUser($id: ID!, $limit: Int) { user(id: $id) }",
            ),
            (Language::Generic, "alone $ and $123 and $"),
            (Language::Rust, "$"),
            (Language::Rust, "$$$$$$"),
            (Language::Rust, ""),
        ];

        for (lang, text) in test_cases {
            let mut highlighter = SyntaxHighlighter::new(lang);
            let dl = DiffLine {
                kind: DiffLineKind::Addition,
                content: format!("+{}", text),
            };
            let start = std::time::Instant::now();
            let line = highlighter.highlight_line(&dl);
            assert!(
                start.elapsed() < std::time::Duration::from_millis(50),
                "Tokenization took too long, possible infinite loop on: {}",
                text
            );
            let joined: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
            assert_eq!(joined, format!("+{}", text));
        }
    }

    #[test]
    fn test_span_concatenation_exact_reproduction() {
        let lines = [
            (DiffLineKind::Header, "diff --git a/foo.rs b/foo.rs"),
            (
                DiffLineKind::Header,
                "commit 0123456789abcdef0123456789abcdef01234567",
            ),
            (DiffLineKind::Header, "Author: Alice <alice@example.com>"),
            (
                DiffLineKind::Header,
                "Date:   Fri Sep 11 07:00:00 2026 +0700",
            ),
            (DiffLineKind::HunkHeader, "@@ -1,5 +1,6 @@ pub fn test() {"),
            (
                DiffLineKind::Addition,
                "+    let x = 42; // addition with spaces and symbols: !@#$%^&*()",
            ),
            (DiffLineKind::Deletion, "-    let x = 0;"),
            (DiffLineKind::Context, "     println!(\"{}\", x);"),
            (
                DiffLineKind::Normal,
                "Some regular text with arbitrary characters: \t\r\n 123!",
            ),
        ];

        let mut highlighter = SyntaxHighlighter::new(Language::Rust);
        for (kind, content) in lines {
            let dl = DiffLine {
                kind,
                content: content.to_string(),
            };
            let rendered = highlighter.highlight_line(&dl);
            let joined: String = rendered.spans.iter().map(|s| s.content.as_ref()).collect();
            assert_eq!(
                joined, content,
                "Concatenation must match original content exactly"
            );
        }
    }

    #[test]
    fn test_unknown_file_type_resets_language() {
        let mut highlighter = SyntaxHighlighter::new(Language::Rust);
        assert_eq!(highlighter.current_lang, Language::Rust);

        // Header for unknown file type
        let dl = DiffLine {
            kind: DiffLineKind::Header,
            content: "diff --git a/notes.xyz b/notes.xyz".to_string(),
        };
        highlighter.highlight_line(&dl);
        assert_eq!(highlighter.current_lang, Language::Generic);
    }

    #[test]
    fn test_quoted_diff_paths_and_spaces() {
        let mut highlighter = SyntaxHighlighter::new(Language::Generic);
        let dl = DiffLine {
            kind: DiffLineKind::Header,
            content:
                "diff --git \"a/my special dir/file name.py\" \"b/my special dir/file name.py\""
                    .to_string(),
        };
        highlighter.highlight_line(&dl);
        assert_eq!(highlighter.current_lang, Language::Python);
    }

    #[test]
    fn test_multiline_block_comments_and_strings() {
        let mut highlighter = SyntaxHighlighter::new(Language::Rust);
        let l1 = DiffLine {
            kind: DiffLineKind::Normal,
            content: "/* start block comment".to_string(),
        };
        highlighter.highlight_line(&l1);
        assert!(highlighter.in_block_comment);

        let l2 = DiffLine {
            kind: DiffLineKind::Normal,
            content: "   still in block comment".to_string(),
        };
        let line2 = highlighter.highlight_line(&l2);
        assert_eq!(line2.spans[0].style.fg, Some(Color::Rgb(110, 118, 130))); // style_comment
        assert!(highlighter.in_block_comment);

        let l3 = DiffLine {
            kind: DiffLineKind::Normal,
            content: "   end block comment */ let x = 1;".to_string(),
        };
        let line3 = highlighter.highlight_line(&l3);
        assert!(!highlighter.in_block_comment);
        let joined: String = line3.spans.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(joined, l3.content);
    }
}

/// Mapping between LeetCode language slugs and source file extensions.
///
/// The slug is what the API expects in `lang` fields; the extension is used
/// when generating solution files.
const LANGS: &[(&str, &str)] = &[
    ("cpp", "cpp"),
    ("java", "java"),
    ("python", "py"),
    ("python3", "py"),
    ("c", "c"),
    ("csharp", "cs"),
    ("javascript", "js"),
    ("typescript", "ts"),
    ("php", "php"),
    ("swift", "swift"),
    ("kotlin", "kt"),
    ("dart", "dart"),
    ("golang", "go"),
    ("go", "go"),
    ("ruby", "rb"),
    ("scala", "scala"),
    ("rust", "rs"),
    ("racket", "rkt"),
    ("erlang", "erl"),
    ("elixir", "ex"),
    ("mysql", "sql"),
];

/// Curated, ordered list of language slugs offered in the TUI language picker.
/// (A subset of `LANGS` that drops near-duplicate slugs like `python`/`go`.)
pub const PICKABLE: &[&str] = &[
    "python3",
    "cpp",
    "java",
    "c",
    "csharp",
    "javascript",
    "typescript",
    "go",
    "rust",
    "kotlin",
    "swift",
    "ruby",
    "scala",
    "php",
    "dart",
    "elixir",
    "erlang",
    "racket",
    "mysql",
];

/// File extension for a language slug (defaults to `txt` if unknown).
pub fn extension_for(lang_slug: &str) -> &'static str {
    LANGS
        .iter()
        .find(|(slug, _)| *slug == lang_slug)
        .map(|(_, ext)| *ext)
        .unwrap_or("txt")
}

/// Best-effort guess of a language slug from a file extension.
pub fn slug_from_extension(ext: &str) -> Option<&'static str> {
    match ext {
        "cpp" | "cc" | "cxx" => Some("cpp"),
        "java" => Some("java"),
        "py" => Some("python3"),
        "c" => Some("c"),
        "cs" => Some("csharp"),
        "js" => Some("javascript"),
        "ts" => Some("typescript"),
        "php" => Some("php"),
        "swift" => Some("swift"),
        "kt" => Some("kotlin"),
        "dart" => Some("dart"),
        "go" => Some("golang"),
        "rb" => Some("ruby"),
        "scala" => Some("scala"),
        "rs" => Some("rust"),
        "rkt" => Some("racket"),
        "erl" => Some("erlang"),
        "ex" => Some("elixir"),
        "sql" => Some("mysql"),
        _ => None,
    }
}

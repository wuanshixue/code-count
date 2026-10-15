//! Pure line-counting core. No UI dependency, so it is unit-testable.

use std::collections::HashMap;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

/// Directories that are skipped by default (build artifacts / VCS / caches).
pub const DEFAULT_SKIP_DIRS: &[&str] = &[
    ".git",
    ".hg",
    ".svn",
    "node_modules",
    "target",
    "dist",
    "build",
    "out",
    ".next",
    ".nuxt",
    ".venv",
    "venv",
    "__pycache__",
    ".mypy_cache",
    ".pytest_cache",
    ".ruff_cache",
    ".cache",
    "vendor",
    ".gradle",
    ".idea",
    ".vs",
    "bin",
    "obj",
];

/// A file whose first bytes contain NUL is treated as binary and skipped.
const BINARY_SNIFF_BYTES: usize = 8192;
/// Files larger than this are skipped (guards against a stray .iso / .vmdk).
const MAX_FILE_BYTES: u64 = 64 * 1024 * 1024;

#[derive(Clone, Debug)]
pub struct Options {
    pub skip_dirs: Vec<String>,
    /// If non-empty, only files with one of these extensions are counted.
    pub include_exts: Vec<String>,
    pub follow_symlinks: bool,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            skip_dirs: DEFAULT_SKIP_DIRS.iter().map(|s| s.to_string()).collect(),
            include_exts: Vec::new(),
            follow_symlinks: false,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ExtStat {
    pub files: u64,
    pub lines: u64,
    pub bytes: u64,
}

#[derive(Clone, Debug, Default)]
pub struct Report {
    pub files: u64,
    pub lines: u64,
    pub bytes: u64,
    /// Files skipped because they looked binary.
    pub binary_files: u64,
    /// Files matched by an extension in `include_exts` exclusion (only meaningful then).
    pub filtered_files: u64,
    /// Directories pruned by `skip_dirs`.
    pub skipped_dirs: u64,
    /// Paths that could not be read (permission denied, deleted mid-scan, ...).
    pub errors: u64,
    /// Extension -> stats. Key `""` means "no extension".
    pub by_ext: HashMap<String, ExtStat>,
}

impl Report {
    /// Extensions sorted by descending line count.
    pub fn ranked(&self) -> Vec<(String, ExtStat)> {
        let mut v: Vec<_> = self.by_ext.iter().map(|(k, s)| (k.clone(), *s)).collect();
        v.sort_by(|a, b| {
            b.1
                .lines
                .cmp(&a.1.lines)
                .then_with(|| b.1.files.cmp(&a.1.files))
                .then_with(|| a.0.cmp(&b.0))
        });
        v
    }
}

#[derive(Clone, Debug)]
pub struct Progress {
    pub files: u64,
    pub lines: u64,
    pub current: PathBuf,
}

/// Lowercase extension of `p`, without the dot. Empty when there is none.
pub fn ext_key(p: &Path) -> String {
    p.extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .unwrap_or_default()
}

/// Count lines the same way `Get-Content | Measure-Object -Line` does:
/// number of `\n` bytes, plus one trailing line when the file does not end in a newline.
pub fn count_lines_in<R: BufRead>(mut r: R) -> std::io::Result<(u64, u64, bool)> {
    let mut lines = 0u64;
    let mut bytes = 0u64;
    let mut buf = vec![0u8; 64 * 1024];
    let mut last_byte: Option<u8> = None;
    loop {
        let n = r.read(&mut buf)?;
        if n == 0 {
            break;
        }
        let chunk = &buf[..n];
        // 只在前 BINARY_SNIFF_BYTES 字节内嗅探 NUL，避免误伤大型文本文件。
        if bytes < BINARY_SNIFF_BYTES as u64 {
            let window = (BINARY_SNIFF_BYTES as u64 - bytes).min(n as u64) as usize;
            if chunk[..window].contains(&0) {
                return Ok((0, bytes + n as u64, true)); // binary
            }
        }
        bytes += n as u64;
        lines += chunk.iter().filter(|&&b| b == b'\n').count() as u64;
        last_byte = Some(chunk[n - 1]);
    }
    if bytes > 0 && last_byte != Some(b'\n') {
        lines += 1;
    }
    Ok((lines, bytes, false))
}

/// Recursively count `root`. `on_progress` is called every `PROGRESS_EVERY` files.
pub fn count_dir<F>(root: &Path, opts: &Options, mut on_progress: F) -> Report
where
    F: FnMut(&Progress),
{
    const PROGRESS_EVERY: u64 = 64;

    let mut report = Report::default();

    let skip: Vec<String> = opts
        .skip_dirs
        .iter()
        .map(|s| s.to_ascii_lowercase())
        .collect();

    let mut skipped_dirs = 0u64;
    let walker = WalkDir::new(root)
        .follow_links(opts.follow_symlinks)
        .into_iter()
        .filter_entry(|e| {
            if e.depth() > 0 && e.file_type().is_dir() {
                let name = e.file_name().to_string_lossy().to_ascii_lowercase();
                if skip.contains(&name) {
                    skipped_dirs += 1;
                    return false;
                }
            }
            true
        });

    for entry in walker {
        let entry = match entry {
            Ok(e) => e,
            Err(_) => {
                report.errors += 1;
                continue;
            }
        };
        if entry.depth() == 0 {
            continue;
        }
        if !entry.file_type().is_file() {
            continue;
        }

        let path = entry.path();
        let ext = ext_key(path);
        if !opts.include_exts.is_empty() {
            let wanted = opts
                .include_exts
                .iter()
                .any(|w| w.trim_start_matches('.').eq_ignore_ascii_case(&ext));
            if !wanted {
                report.filtered_files += 1;
                continue;
            }
        }

        match entry.metadata() {
            Ok(m) if m.len() > MAX_FILE_BYTES => {
                report.filtered_files += 1;
                continue;
            }
            _ => {}
        }

        let file = match File::open(path) {
            Ok(f) => f,
            Err(_) => {
                report.errors += 1;
                continue;
            }
        };

        match count_lines_in(BufReader::new(file)) {
            Ok((lines, bytes, binary)) => {
                if binary {
                    report.binary_files += 1;
                    continue;
                }
                report.files += 1;
                report.lines += lines;
                report.bytes += bytes;
                let stat = report.by_ext.entry(ext).or_default();
                stat.files += 1;
                stat.lines += lines;
                stat.bytes += bytes;

                if report.files % PROGRESS_EVERY == 0 {
                    on_progress(&Progress {
                        files: report.files,
                        lines: report.lines,
                        current: path.to_path_buf(),
                    });
                }
            }
            Err(_) => report.errors += 1,
        }
    }

    report.skipped_dirs = skipped_dirs;
    on_progress(&Progress {
        files: report.files,
        lines: report.lines,
        current: root.to_path_buf(),
    });
    report
}

/// Human-readable byte size.
pub fn human_bytes(n: u64) -> String {
    const U: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut v = n as f64;
    let mut i = 0;
    while v >= 1024.0 && i < U.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    if i == 0 {
        format!("{n} B")
    } else {
        format!("{v:.1} {}", U[i])
    }
}

/// Thousands separator, e.g. `1234567` -> `1,234,567`.
pub fn thousands(n: u64) -> String {
    let s = n.to_string();
    let mut out = String::with_capacity(s.len() + s.len() / 3);
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn lines(s: &str) -> u64 {
        count_lines_in(Cursor::new(s.as_bytes().to_vec())).unwrap().0
    }

    #[test]
    fn no_trailing_newline_counts_last_line() {
        assert_eq!(lines("a"), 1);
        assert_eq!(lines("a\nb"), 2);
        assert_eq!(lines("a\nb\n"), 2);
        assert_eq!(lines("a\nb\n\n"), 3);
    }

    #[test]
    fn empty_file_is_zero_lines() {
        assert_eq!(lines(""), 0);
    }

    #[test]
    fn crlf_counts_lines_not_bytes() {
        assert_eq!(lines("a\r\nb\r\n"), 2);
    }

    #[test]
    fn binary_is_detected() {
        let data = vec![b'a', 0u8, b'b'];
        let (_, _, binary) = count_lines_in(Cursor::new(data)).unwrap();
        assert!(binary);
    }

    #[test]
    fn utf8_multibyte_does_not_inflate_line_count() {
        assert_eq!(lines("你好世界\n再见\n"), 2);
        assert_eq!(lines("🎉🎉\n"), 1);
    }

    #[test]
    fn human_bytes_formats() {
        assert_eq!(human_bytes(512), "512 B");
        assert_eq!(human_bytes(2048), "2.0 KB");
    }

    #[test]
    fn thousands_separates() {
        assert_eq!(thousands(7), "7");
        assert_eq!(thousands(1234), "1,234");
        assert_eq!(thousands(1234567), "1,234,567");
    }

    #[test]
    fn ext_key_lowercases_and_strips_dot() {
        assert_eq!(ext_key(Path::new("a/B.RS")), "rs");
        assert_eq!(ext_key(Path::new("a/Makefile")), "");
    }

    #[test]
    fn count_dir_groups_by_ext_and_skips_dirs() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::create_dir_all(root.join("node_modules/pkg")).unwrap();
        std::fs::write(root.join("src/a.rs"), "fn a() {}\nfn b() {}\n").unwrap();
        std::fs::write(root.join("src/b.rs"), "fn c() {}\n").unwrap();
        std::fs::write(root.join("main.py"), "print(1)\n").unwrap();
        std::fs::write(root.join("node_modules/pkg/i.js"), "x\n".repeat(100)).unwrap();

        let report = count_dir(root, &Options::default(), |_| {});
        assert_eq!(report.files, 3, "node_modules must be pruned");
        assert_eq!(report.lines, 4);
        assert_eq!(report.by_ext["rs"].lines, 3);
        assert_eq!(report.by_ext["rs"].files, 2);
        assert_eq!(report.by_ext["py"].lines, 1);

        let ranked = report.ranked();
        assert_eq!(ranked[0].0, "rs");
    }

    #[test]
    fn include_exts_filters() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        std::fs::write(root.join("a.rs"), "1\n").unwrap();
        std::fs::write(root.join("b.py"), "1\n2\n").unwrap();

        let opts = Options {
            include_exts: vec!["rs".into()],
            ..Default::default()
        };
        let report = count_dir(root, &opts, |_| {});
        assert_eq!(report.files, 1);
        assert_eq!(report.lines, 1);
        assert_eq!(report.filtered_files, 1);
    }
}

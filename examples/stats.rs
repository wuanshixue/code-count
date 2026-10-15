//! 命令行版本，用来和 PowerShell 的结果互相核对。
//!
//!   cargo run --example stats -- <目录>
//!
//! 对照命令：
//!   (Get-ChildItem -Recurse -File | Get-Content | Measure-Object).Count

use std::path::PathBuf;

use code_count::count::{Options, human_bytes, thousands};

fn main() {
    let Some(arg) = std::env::args().nth(1) else {
        eprintln!("用法: stats <目录>");
        std::process::exit(2);
    };
    let root = PathBuf::from(arg);

    let opts = Options::default();
    let report = code_count::count::count_dir(&root, &opts, |_| {});

    println!("目录: {}", root.display());
    println!("文件: {}", thousands(report.files));
    println!("行数: {}", thousands(report.lines));
    println!("大小: {}", human_bytes(report.bytes));
    println!("跳过目录: {}", report.skipped_dirs);
    println!("二进制文件: {}", report.binary_files);
    println!("读取失败: {}", report.errors);
    println!();
    println!("{:<16} {:>8} {:>12} {:>10}", "扩展名", "文件", "行数", "大小");
    for (ext, s) in report.ranked() {
        let name = if ext.is_empty() { "(无)" } else { ext.as_str() };
        println!(
            "{:<16} {:>8} {:>12} {:>10}",
            name,
            thousands(s.files),
            thousands(s.lines),
            human_bytes(s.bytes)
        );
    }
}

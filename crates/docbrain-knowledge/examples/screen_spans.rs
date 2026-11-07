// SPDX-License-Identifier: MIT
//! The open secret screen over a stream, for a differential against another screen: reads `id<TAB>text` lines
//! on stdin, the text escaped as PostgreSQL's COPY text format writes it (`\n`, `\t`, `\\`, ...), and prints
//! `H<TAB>id<TAB>rule<TAB>start<TAB>end` per hit, byte offsets into the unescaped text, then `R<TAB>id` per
//! line. It never prints text.

use std::io::{BufRead, Write};

/// COPY text-format unescaping (`\N`, a null, is the empty text).
fn unescape(f: &str) -> String {
    if f == "\\N" {
        return String::new();
    }
    let mut out = String::with_capacity(f.len());
    let mut it = f.chars();
    while let Some(c) = it.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match it.next() {
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('r') => out.push('\r'),
            Some('b') => out.push('\u{8}'),
            Some('f') => out.push('\u{c}'),
            Some('v') => out.push('\u{b}'),
            Some(o) => out.push(o),
            None => {}
        }
    }
    out
}

fn main() -> std::io::Result<()> {
    let stdin = std::io::stdin();
    let mut out = std::io::BufWriter::new(std::io::stdout().lock());
    for line in stdin.lock().lines() {
        let line = line?;
        let Some((id, text)) = line.split_once('\t') else { continue };
        for (rule, at) in docbrain_knowledge::secret::spans(&unescape(text)) {
            writeln!(out, "H\t{id}\t{rule}\t{}\t{}", at.start, at.end)?;
        }
        writeln!(out, "R\t{id}")?;
    }
    out.flush()
}

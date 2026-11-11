// SPDX-License-Identifier: MIT
//! Guard: the CLI reads the server's event stream through `TextStream` (`src/stream_text.rs`),
//! which decodes across the cuts the network makes in a stream and gives the parser one line
//! end. A chunk decoded on its own turns a character that straddles two chunks into two U+FFFD,
//! which is how an answer with an accent or an emoji in it came out damaged.

use std::path::{Path, PathBuf};

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for e in entries.flatten() {
        let p = e.path();
        if p.is_dir() {
            rust_files(&p, out);
        } else if p.extension().is_some_and(|x| x == "rs") {
            out.push(p);
        }
    }
}

#[test]
fn every_byte_stream_is_read_through_text_stream() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = Vec::new();
    rust_files(&src, &mut files);
    assert!(files.iter().any(|f| f.ends_with("src/main.rs")), "no sources were found: {files:?}");

    // Wherever a response is read as a stream of bytes, the stream is wrapped where it is made,
    // so its chunks can only be had as text that was decoded across the cuts.
    let mut wrapped = Vec::new();
    let mut bare = Vec::new();
    for f in files.iter().filter(|f| !f.ends_with("src/stream_text.rs")) {
        let text = std::fs::read_to_string(f).unwrap();
        for (i, line) in text.lines().enumerate() {
            if line.trim_start().starts_with("//") || !line.contains(".bytes_stream()") {
                continue;
            }
            let at = format!("{}:{}", f.strip_prefix(&src).unwrap_or(f).display(), i + 1);
            if line.contains("TextStream::new(") { wrapped.push(at) } else { bare.push(at) }
        }
    }
    assert!(bare.is_empty(), "a byte stream is read without TextStream::new around it: {bare:?}");
    assert_eq!(wrapped.len(), 1, "the one reader of the server's event stream is handle_sse_stream: {wrapped:?}");

    // And that reader turns no bytes into text itself.
    let main = std::fs::read_to_string(src.join("main.rs")).unwrap();
    let (_, after) = main.split_once("\nasync fn handle_sse_stream(").expect("handle_sse_stream is in main.rs");
    let reader: Vec<&str> = after.lines().skip(1).take_while(|line| !line.starts_with("fn ") && !line.starts_with("async fn ")).collect();
    let reader = reader.join("\n");
    assert!(reader.contains("TextStream::new("), "handle_sse_stream does not read through TextStream");
    assert!(!reader.contains("from_utf8"), "handle_sse_stream turns bytes into text itself");
}

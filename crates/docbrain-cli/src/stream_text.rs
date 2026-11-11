// SPDX-License-Identifier: MIT
//! Text from a stream of bytes, read the same wherever the network cut it.
//!
//! A streamed reply arrives in chunks, and a chunk can end anywhere: inside a character, or
//! between the two bytes of a `\r\n` line end. Decoding each chunk on its own turns a character
//! that straddles two chunks into two U+FFFD. [`TextStream`] decodes across the cut instead.
//!
//! It also gives every reader one line end. An event stream may end its lines with `\n`, `\r\n`
//! or a lone `\r`, and a gateway on the way may rewrite one into another; a reader that looks for
//! `\n` alone then finds no line, or no event, at all. Here `\r\n` and a lone `\r` both come out
//! as `\n`. Only raw carriage returns are line ends: one written inside JSON is the two characters
//! `\` and `r`, and passes through untouched.

use futures_util::{Stream, StreamExt};

/// The bytes of a stream, chunk by chunk, as text.
#[derive(Default)]
struct StreamText {
    /// The first bytes of a character whose last bytes have not arrived: three at most.
    tail: Vec<u8>,
    /// The last character read was `\r`. A `\n` that comes next is the second half of the same
    /// line end, even when it comes in the next chunk.
    after_cr: bool,
}

impl StreamText {
    /// The text `chunk` adds. Bytes that are not UTF-8 become U+FFFD, exactly where
    /// `String::from_utf8_lossy` would put them for the stream as a whole; the first bytes of a
    /// character that the chunk leaves unfinished wait for the next chunk.
    fn push(&mut self, chunk: &[u8]) -> String {
        let mut bytes = std::mem::take(&mut self.tail);
        bytes.extend_from_slice(chunk);
        let mut out = String::with_capacity(bytes.len());
        let mut rest = bytes.as_slice();
        loop {
            match std::str::from_utf8(rest) {
                Ok(text) => {
                    self.emit(text, &mut out);
                    return out;
                }
                Err(error) => {
                    let (valid, after) = rest.split_at(error.valid_up_to());
                    self.emit(&String::from_utf8_lossy(valid), &mut out);
                    match error.error_len() {
                        // Bytes that can never be a character: one U+FFFD, and on with the rest.
                        Some(invalid) => {
                            self.emit("\u{FFFD}", &mut out);
                            rest = after.get(invalid..).unwrap_or_default();
                        }
                        // The chunk ends inside a character.
                        None => {
                            self.tail = after.to_vec();
                            return out;
                        }
                    }
                }
            }
        }
    }

    /// What the end of the stream leaves: a character cut off for good reads as U+FFFD.
    fn finish(&mut self) -> String {
        let tail = std::mem::take(&mut self.tail);
        let mut out = String::new();
        self.emit(&String::from_utf8_lossy(&tail), &mut out);
        out
    }

    /// Appends `text` with its line ends as `\n`. A `\r` is read as a line end the moment it
    /// arrives, so a reader never waits for the next chunk to see a line that is already whole;
    /// the `\n` of a `\r\n` pair then adds nothing, even when the network cut the pair in two.
    fn emit(&mut self, text: &str, out: &mut String) {
        for c in text.chars() {
            match c {
                '\n' if self.after_cr => {}
                '\r' => out.push('\n'),
                c => out.push(c),
            }
            self.after_cr = c == '\r';
        }
    }
}

/// A stream of byte chunks read as text: one piece for each chunk, then whatever the end of the
/// stream leaves over.
pub(crate) struct TextStream<S> {
    chunks: S,
    text: StreamText,
    ended: bool,
}

impl<S, B, E> TextStream<S>
where
    S: Stream<Item = Result<B, E>> + Unpin,
    B: AsRef<[u8]>,
{
    pub(crate) fn new(chunks: S) -> Self {
        Self { chunks, text: StreamText::default(), ended: false }
    }

    /// The next piece of text, an error of the stream as it came, or `None` once the stream has
    /// ended. A piece can be empty: its chunk only continued a character. An ended stream is not
    /// asked again.
    pub(crate) async fn next(&mut self) -> Option<Result<String, E>> {
        if self.ended {
            return None;
        }
        match self.chunks.next().await {
            Some(Ok(chunk)) => Some(Ok(self.text.push(chunk.as_ref()))),
            Some(Err(error)) => Some(Err(error)),
            None => {
                self.ended = true;
                Some(self.text.finish()).filter(|rest| !rest.is_empty()).map(Ok)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures_util::stream;
    use proptest::prelude::*;
    use std::convert::Infallible;

    /// What a reader must end up with, written without the step under test: the whole stream
    /// decoded at once, then its line ends made `\n`.
    fn read_whole(bytes: &[u8]) -> String {
        String::from_utf8_lossy(bytes).replace("\r\n", "\n").replace('\r', "\n")
    }

    /// The same bytes read in pieces, cut at each of `cuts`.
    fn read_in_pieces(bytes: &[u8], cuts: &[usize]) -> String {
        let mut cuts: Vec<usize> = cuts.iter().map(|cut| (*cut).min(bytes.len())).collect();
        cuts.sort_unstable();
        let mut pieces: Vec<&[u8]> = Vec::new();
        let mut from = 0;
        for cut in cuts {
            pieces.push(&bytes[from..cut]);
            from = cut;
        }
        pieces.push(&bytes[from..]);
        read_chunks(&pieces)
    }

    /// The chunks read one after another, then the end of the stream. What waits between two
    /// chunks is never more than the three bytes an unfinished character can have.
    fn read_chunks(chunks: &[&[u8]]) -> String {
        let mut text = StreamText::default();
        let mut out = String::new();
        for chunk in chunks {
            out.push_str(&text.push(chunk));
            assert!(text.tail.len() <= 3, "{} bytes wait after {chunk:?}", text.tail.len());
        }
        out.push_str(&text.finish());
        assert!(text.tail.is_empty(), "the end of the stream leaves nothing waiting");
        out
    }

    /// Characters of one, two, three and four bytes, the three line ends, and stray bytes: a lone
    /// byte of any value, and a character with its last byte missing.
    fn part() -> impl Strategy<Value = Vec<u8>> {
        prop_oneof![
            Just(b"a".to_vec()),
            Just("é".as_bytes().to_vec()),
            Just("東".as_bytes().to_vec()),
            Just("🚀".as_bytes().to_vec()),
            any::<char>().prop_map(|c| c.to_string().into_bytes()),
            Just(b"\n".to_vec()),
            Just(b"\r\n".to_vec()),
            Just(b"\r".to_vec()),
            any::<u8>().prop_map(|byte| vec![byte]),
            any::<char>().prop_map(|c| {
                let mut bytes = c.to_string().into_bytes();
                bytes.pop();
                bytes
            }),
        ]
    }

    fn bytes() -> impl Strategy<Value = Vec<u8>> {
        prop::collection::vec(part(), 0..40).prop_map(|parts| parts.concat())
    }

    fn string() -> impl Strategy<Value = String> {
        bytes().prop_map(|bytes| String::from_utf8_lossy(&bytes).replace('\r', ""))
    }

    fn cuts() -> impl Strategy<Value = Vec<prop::sample::Index>> {
        prop::collection::vec(any::<prop::sample::Index>(), 0..6)
    }

    fn offsets(cuts: &[prop::sample::Index], len: usize) -> Vec<usize> {
        cuts.iter().map(|cut| cut.index(len + 1)).collect()
    }

    proptest! {
        /// Any string, cut into chunks at any bytes, decodes to itself.
        #[test]
        fn a_string_cut_at_any_bytes_decodes_to_itself(text in string(), cuts in cuts()) {
            let cuts = offsets(&cuts, text.len());
            prop_assert_eq!(read_in_pieces(text.as_bytes(), &cuts), text);
        }

        /// Bytes that are not UTF-8 decode as `String::from_utf8_lossy` decodes the whole of them.
        #[test]
        fn bytes_that_are_not_text_decode_as_the_whole_would(bytes in bytes(), cuts in cuts()) {
            let bytes: Vec<u8> = bytes.into_iter().filter(|byte| *byte != b'\r').collect();
            let cuts = offsets(&cuts, bytes.len());
            prop_assert_eq!(read_in_pieces(&bytes, &cuts), String::from_utf8_lossy(&bytes).into_owned());
        }

        /// Any mix of `\n`, `\r\n` and `\r`, cut at any bytes, reads as the whole with its line
        /// ends made `\n`.
        #[test]
        fn line_ends_cut_at_any_bytes_read_as_the_normalised_whole(bytes in bytes(), cuts in cuts()) {
            let cuts = offsets(&cuts, bytes.len());
            prop_assert_eq!(read_in_pieces(&bytes, &cuts), read_whole(&bytes));
        }

        /// Text that has been through the step once is not changed by going through it again.
        #[test]
        fn text_already_read_reads_as_itself(bytes in bytes(), cuts in cuts()) {
            let once = read_whole(&bytes);
            let cuts = offsets(&cuts, once.len());
            prop_assert_eq!(read_in_pieces(once.as_bytes(), &cuts), once);
        }
    }

    /// A text with every kind of trouble in it: characters of each width, each line end, a stray
    /// byte, a character cut short before a line end and one cut short by the end of the stream.
    fn known_bytes() -> Vec<u8> {
        let mut bytes = "aé東🚀\r\nb\rc\n\r\n\r\r\nz".as_bytes().to_vec();
        bytes.extend_from_slice(&[0xFF, b'\r', 0xE6, 0x9D, b'\n', 0xF0, 0x9F]);
        bytes
    }

    /// Every pair of cuts of a known text, none left out, reads as the whole.
    #[test]
    fn every_pair_of_cuts_of_a_known_text_reads_the_same() {
        let bytes = known_bytes();
        let whole = read_whole(&bytes);
        assert_eq!(whole, "aé東🚀\nb\nc\n\n\n\nz\u{FFFD}\n\u{FFFD}\n\u{FFFD}");
        for first in 0..=bytes.len() {
            for second in first..=bytes.len() {
                assert_eq!(read_in_pieces(&bytes, &[first, second]), whole, "cut at bytes {first} and {second}");
            }
        }
    }

    /// The same text with every byte in a chunk of its own.
    #[test]
    fn one_byte_at_a_time_reads_the_same() {
        let bytes = known_bytes();
        let every_offset: Vec<usize> = (0..=bytes.len()).collect();
        assert_eq!(read_in_pieces(&bytes, &every_offset), read_whole(&bytes));
    }

    /// A character cut between any two of its bytes is still one character, and nothing of it is
    /// handed on before its last byte arrives.
    #[test]
    fn a_character_cut_between_its_bytes_is_one_character() {
        for character in ["é", "東", "🚀"] {
            let bytes = character.as_bytes();
            for cut in 1..bytes.len() {
                let mut text = StreamText::default();
                assert_eq!(text.push(&bytes[..cut]), "", "{character} cut at byte {cut}");
                assert_eq!(text.push(&bytes[cut..]), character, "{character} cut at byte {cut}");
                assert_eq!(text.finish(), "");
            }
        }
    }

    /// A `\r\n` the network cut between its two bytes is one line end, not two.
    #[test]
    fn a_line_end_cut_between_its_two_bytes_is_one_line_end() {
        let mut text = StreamText::default();
        let mut out = text.push(b"data: 1\r");
        out.push_str(&text.push(b"\n\r"));
        out.push_str(&text.push(b"\ndata: 2\r\n\r\n"));
        out.push_str(&text.finish());
        assert_eq!(out, "data: 1\n\ndata: 2\n\n");
    }

    /// A `\r` is a line end whatever follows it, so it is read as one the moment it arrives. A
    /// reader is never left waiting on the next chunk for a line, or an event, it already has.
    #[test]
    fn a_line_end_is_read_as_soon_as_its_first_byte_arrives() {
        let mut text = StreamText::default();
        assert_eq!(text.push(b"data: 1\r\n\r"), "data: 1\n\n");
        assert_eq!(text.push(b"\n"), "", "the second half of the pair adds nothing");
        assert_eq!(text.push(b"\n"), "\n", "a line end of its own after it still counts");
        assert_eq!(text.finish(), "");
    }

    /// A character the stream never finishes is one U+FFFD, as it is in a lossy decode of the
    /// whole.
    #[test]
    fn a_character_the_stream_never_finishes_is_one_replacement() {
        for cut_short in [&[0xC3][..], &[0xE6, 0x9D], &[0xF0, 0x9F, 0x9A]] {
            let mut text = StreamText::default();
            assert_eq!(text.push(b"caf"), "caf");
            assert_eq!(text.push(cut_short), "");
            assert_eq!(text.finish(), "\u{FFFD}");
            assert_eq!(text.finish(), "", "the end leaves it once");
        }
    }

    /// Inputs chosen to break the step, each with what it must read as. The expectation is
    /// written by hand and checked against the lossy decode of the whole as well.
    #[test]
    fn inputs_chosen_to_break_it() {
        let cases: &[(&[&[u8]], &str)] = &[
            (&[b""], ""),
            (&[b"", b"", b""], ""),
            (&[&[0xC3], &[0xA9]], "é"),
            (&[&[0xF0], &[0x9F], &[0x9A], &[0x80]], "🚀"),
            (&[&[0xF0, 0x9F], b"", &[0x9A, 0x80]], "🚀"),
            (&[&[0xE6, 0x9D], b"A"], "\u{FFFD}A"),
            (&[&[0xE6], &[0x9D], b"\r", b"\n"], "\u{FFFD}\n"),
            (&[b"\r", &[0xFF], b"\n"], "\n\u{FFFD}\n"),
            (&[b"\r", b"\n"], "\n"),
            (&[b"\r", b"", b"\n"], "\n"),
            (&[b"\n", b"\n"], "\n\n"),
            (&[b"\r", b"\r", b"\n", b"\n"], "\n\n\n"),
            (&[b"\r\n\r", b"\n"], "\n\n"),
            (&[b"a\r"], "a\n"),
            (&[&[0xC0, 0x80]], "\u{FFFD}\u{FFFD}"),
            (&[&[0xED, 0xA0], &[0x80]], "\u{FFFD}\u{FFFD}\u{FFFD}"),
            (&[&[0xF4, 0x90], &[0x80, 0x80]], "\u{FFFD}\u{FFFD}\u{FFFD}\u{FFFD}"),
            (&[&[0xEF, 0xBB], &[0xBF], b"x"], "\u{FEFF}x"),
            (&["\u{2028}\u{85}".as_bytes()], "\u{2028}\u{85}"),
            (&[b"\0\x0b\x0c"], "\0\u{b}\u{c}"),
            (&[br#"{"text":"a\r\nb"}"#], r#"{"text":"a\r\nb"}"#),
            // What waits between two chunks, met by what cannot finish it: a stray byte, another
            // character, another first byte, a letter, a byte no character has there.
            (&[&[0xC3], &[0xFF], &[0xA9]], "\u{FFFD}\u{FFFD}\u{FFFD}"),
            (&[&[0xE6], "é".as_bytes()], "\u{FFFD}é"),
            (&[&[0xE6], &[0xC3]], "\u{FFFD}\u{FFFD}"),
            (&[&[0xF0, 0x9F, 0x9A], b"A"], "\u{FFFD}A"),
            (&[&[0xF0], &[0x80]], "\u{FFFD}\u{FFFD}"),
            (&[&[0xE6, 0x9D], b"\r", &[0xB1]], "\u{FFFD}\n\u{FFFD}"),
            // The first half of a line end, then something that is not its second half.
            (&[b"\r", &[0xC3], b"\n"], "\n\u{FFFD}\n"),
            (&[b"\r", &[0xC3], &[0xA9], b"\n"], "\né\n"),
            (&[b"\r", &[0xC3]], "\n\u{FFFD}"),
            (&[b"\r", &[0xEF, 0xBB, 0xBF], b"\n"], "\n\u{FEFF}\n"),
            (&[b"\n", b"\r"], "\n\n"),
            (&[b"\r", b"\r\n", b"\n"], "\n\n\n"),
            // Every line end of a stream cut between its two bytes, and a blank line cut at each.
            (&[b"\r", b"\n\r", b"\n\r", b"\n"], "\n\n\n"),
            (&[b"a\r", b"\n", b"\r", b"\n", b"b"], "a\n\nb"),
            // Line ends of other standards are characters here, also when a cut falls inside one.
            (&[&[0xC2], &[0x85]], "\u{85}"),
            (&[&[0xE2, 0x80], &[0xA8]], "\u{2028}"),
            // A raw carriage return is a line end wherever it stands, between two JSON tokens too.
            (&[b"{\"a\":1,\r\"b\":2}\n"], "{\"a\":1,\n\"b\":2}\n"),
        ];
        for (chunks, want) in cases {
            assert_eq!(read_chunks(chunks), *want, "{chunks:?}");
            assert_eq!(read_whole(&chunks.concat()), *want, "the expectation written for {chunks:?}");
            println!("{chunks:?} reads as {want:?}");
        }
    }

    /// A megabyte of bytes that are no character, alone and then mixed with letters and with
    /// first bytes nothing finishes: each reads as it would in the whole, and the work grows with
    /// the stream, not with its square.
    #[test]
    fn a_megabyte_of_bytes_that_are_not_text_reads_as_the_whole_would() {
        let stray = vec![0xFF; 1024 * 1024];
        let read = read_chunks(&[stray.as_slice()]);
        assert!(read.chars().count() == stray.len() && read.chars().all(|c| c == '\u{FFFD}'), "a megabyte of stray bytes came out changed");
        let mixed = [b'a', 0xFF, 0xC3].repeat(350_000);
        let pieces: Vec<&[u8]> = mixed.chunks(7).collect();
        assert!(read_chunks(&pieces) == read_whole(&mixed), "a megabyte of mixed bytes came out changed");
    }

    /// A megabyte of four-byte characters in chunks of seven bytes, so that every chunk but one
    /// in four ends inside a character.
    #[test]
    fn a_long_stream_in_small_chunks_reads_the_same() {
        let whole = "🚀".repeat(256 * 1024);
        let mut text = StreamText::default();
        let mut out = String::with_capacity(whole.len());
        for chunk in whole.as_bytes().chunks(7) {
            out.push_str(&text.push(chunk));
        }
        out.push_str(&text.finish());
        assert!(out == whole, "a long stream came out changed");
    }

    /// One piece of text for each chunk, then what the end leaves, then nothing; and a stream that
    /// has ended is not asked again (this one panics if it is).
    #[tokio::test]
    async fn a_text_stream_hands_on_each_chunk_then_what_the_end_leaves() {
        let chunks = vec![b"caf".to_vec(), vec![0xC3], vec![0xA9, b'\r'], vec![b'\n', 0xE6, 0x9D]];
        let mut left = chunks.into_iter();
        let once = Box::pin(stream::unfold((), move |()| {
            let next = left.next();
            async move { next.map(|chunk| (Ok::<_, Infallible>(chunk), ())) }
        }));
        let mut text = TextStream::new(once);
        let mut pieces = Vec::new();
        while let Some(piece) = text.next().await {
            pieces.push(piece.unwrap_or_default());
        }
        assert_eq!(pieces, ["caf", "", "é\n", "", "\u{FFFD}"]);
        assert!(text.next().await.is_none());
    }

    /// An error of the stream comes through as it is, after the text that came before it.
    #[tokio::test]
    async fn an_error_of_the_stream_comes_through_as_it_is() {
        let chunks = stream::iter(vec![Ok(b"a\r".to_vec()), Err("the connection was reset")]);
        let mut text = TextStream::new(chunks);
        assert_eq!(text.next().await, Some(Ok("a\n".to_string())));
        assert_eq!(text.next().await, Some(Err("the connection was reset")));
    }

    /// An error changes nothing the step has read. A caller that reads on after one still gets a
    /// character whose bytes came on either side of it as one character, and a line end whose
    /// bytes did as one line end.
    #[tokio::test]
    async fn an_error_between_two_halves_leaves_both_halves_one() {
        let chunks = stream::iter(vec![Ok(vec![b'a', 0xC3]), Err("first"), Ok(vec![0xA9, b'\r']), Err("second"), Ok(vec![b'\n', b'b'])]);
        let mut text = TextStream::new(chunks);
        let mut pieces = Vec::new();
        while let Some(piece) = text.next().await {
            pieces.push(piece);
        }
        assert_eq!(pieces, [Ok("a".to_string()), Err("first"), Ok("é\n".to_string()), Err("second"), Ok("b".to_string())]);
    }
}

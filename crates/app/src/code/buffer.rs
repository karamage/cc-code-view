//! 表示用のテキスト。行の開始位置だけを持ち、100 万行でも軽く開ける。

const MAX_BYTES: usize = 256 * 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BufferKind {
    Text,
    Binary,
    TooLarge,
}

pub struct Buffer {
    text: String,
    starts: Vec<u32>,
    kind: BufferKind,
    bytes: u64,
}

impl Buffer {
    pub fn from_bytes(bytes: Vec<u8>) -> Buffer {
        let len = bytes.len() as u64;
        if bytes.len() > MAX_BYTES {
            return Buffer { text: String::new(), starts: vec![0], kind: BufferKind::TooLarge, bytes: len };
        }
        if bytes[..bytes.len().min(8192)].contains(&0) {
            return Buffer { text: String::new(), starts: vec![0], kind: BufferKind::Binary, bytes: len };
        }
        let text = match String::from_utf8(bytes) {
            Ok(t) => t,
            Err(e) => String::from_utf8_lossy(e.as_bytes()).into_owned(),
        };
        let mut starts = Vec::with_capacity(text.len() / 32 + 1);
        starts.push(0);
        starts.extend(text.bytes().enumerate().filter(|(_, b)| *b == b'\n').map(|(i, _)| i as u32 + 1));
        Buffer { text, starts, kind: BufferKind::Text, bytes: len }
    }

    pub fn kind(&self) -> BufferKind {
        self.kind
    }

    pub fn bytes(&self) -> u64 {
        self.bytes
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn line_count(&self) -> usize {
        self.starts.len()
    }

    pub fn line(&self, i: usize) -> &str {
        let start = self.starts[i] as usize;
        let end = self.starts.get(i + 1).map_or(self.text.len(), |n| *n as usize - 1);
        self.text[start..end].trim_end_matches('\r')
    }
}

/// 指定位置の識別子を返す。
pub fn word_at(line: &str, byte: usize) -> Option<&str> {
    let is_word = |c: char| c.is_ascii_alphanumeric() || c == '_';
    if byte > line.len() || !line.is_char_boundary(byte) {
        return None;
    }
    let start = line[..byte].rfind(|c| !is_word(c)).map_or(0, |i| {
        i + line[i..].chars().next().map_or(1, char::len_utf8)
    });
    let end = line[byte..].find(|c| !is_word(c)).map_or(line.len(), |i| byte + i);
    (end > start).then(|| &line[start..end])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn buffer_indexes_lines_and_handles_invalid_utf8() {
        let b = Buffer::from_bytes(b"a\nb\xff\nc".to_vec());
        assert_eq!(b.line_count(), 3);
        assert_eq!(b.line(0), "a");
        assert_eq!(b.line(2), "c");
        assert_eq!(b.kind(), BufferKind::Text);
    }

    #[test]
    fn crlf_is_trimmed() {
        assert_eq!(Buffer::from_bytes(b"a\r\nb".to_vec()).line(0), "a");
    }

    #[test]
    fn buffer_detects_binary() {
        assert_eq!(Buffer::from_bytes(vec![0, 1, 2]).kind(), BufferKind::Binary);
    }

    #[test]
    fn million_lines_open_quickly() {
        let bytes = "x\n".repeat(1_000_000).into_bytes();
        let start = std::time::Instant::now();
        let b = Buffer::from_bytes(bytes);
        assert_eq!(b.line_count(), 1_000_001);
        // デバッグビルドでも十分に速いこと
        assert!(start.elapsed().as_millis() < 1000);
    }

    #[test]
    fn word_at_handles_identifiers_and_multibyte() {
        assert_eq!(word_at("let foo_bar = 1;", 6), Some("foo_bar"));
        assert_eq!(word_at("日本語 x", 0), None);
        assert_eq!(word_at("日本語x_y", 9), Some("x_y"));
        assert_eq!(word_at("  ", 1), None);
    }
}

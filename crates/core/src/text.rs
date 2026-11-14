//! 日本語の段落を、禁則処理をして行に分ける。gpui の折り返しは「。」を行頭に置いてしまう。
//! 文字幅は呼び出し側が測る。画面の段落と図のラベルで同じ規則を使う。

/// 行頭に来てはいけない文字。
pub fn no_line_start(c: char) -> bool {
    matches!(
        c,
        '。' | '、'
            | '，'
            | '．'
            | '）'
            | ')'
            | '」'
            | '』'
            | '】'
            | '］'
            | ']'
            | '｝'
            | '}'
            | '〉'
            | '》'
            | '！'
            | '？'
            | '!'
            | '?'
            | '：'
            | '；'
            | ':'
            | ';'
            | ','
            | '.'
            | 'ー'
            | '・'
            | '…'
            | 'ぁ'
            | 'ぃ'
            | 'ぅ'
            | 'ぇ'
            | 'ぉ'
            | 'っ'
            | 'ゃ'
            | 'ゅ'
            | 'ょ'
            | 'ァ'
            | 'ィ'
            | 'ゥ'
            | 'ェ'
            | 'ォ'
            | 'ッ'
            | 'ャ'
            | 'ュ'
            | 'ョ'
            | '々'
    )
}

/// 行末に来てはいけない文字。
pub fn no_line_end(c: char) -> bool {
    matches!(c, '（' | '(' | '「' | '『' | '【' | '［' | '[' | '｛' | '{' | '〈' | '《')
}

/// 途中で切らない欧文の語の文字。
fn is_word(c: char) -> bool {
    c.is_ascii_alphanumeric()
        || matches!(c, '_' | '-' | '/' | '.' | '\'' | ':' | '$' | '%' | '#' | '@' | '~')
        // カタカナ語も途中で切らない（「ディ/レクトリ」を避ける）
        || matches!(c, 'ァ'..='ヶ' | 'ー')
}

fn is_kanji(c: char) -> bool {
    matches!(c, '\u{4E00}'..='\u{9FFF}' | '\u{3400}'..='\u{4DBF}' | '々')
}

fn is_hiragana(c: char) -> bool {
    matches!(c, 'ぁ'..='ゖ')
}

/// 文字幅を `advance` で測りながら行に分ける。
pub fn break_lines(text: &str, max_width: f32, advance: &dyn Fn(char) -> f32) -> Vec<String> {
    let mut out = Vec::new();
    for paragraph in text.split('\n') {
        break_paragraph(paragraph, max_width, advance, &mut out);
    }
    out
}

fn break_paragraph(
    text: &str,
    max_width: f32,
    advance: &dyn Fn(char) -> f32,
    out: &mut Vec<String>,
) {
    // 文節に近い塊に分ける。漢字やカタカナ、欧文の語に、続くひらがなと句読点をつなげる。
    // 「レビュー / し、」のような読みにくい切れ目を避けるため
    let chars: Vec<char> = text.chars().collect();
    let mut chunks: Vec<String> = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let mut chunk = String::new();
        // 行末禁則の文字は次の文字とつなげる
        while i < chars.len() && no_line_end(chars[i]) {
            chunk.push(chars[i]);
            i += 1;
        }
        if i < chars.len() {
            if is_word(chars[i]) {
                while i < chars.len() && is_word(chars[i]) {
                    chunk.push(chars[i]);
                    i += 1;
                }
            } else if is_kanji(chars[i]) {
                while i < chars.len() && is_kanji(chars[i]) {
                    chunk.push(chars[i]);
                    i += 1;
                }
            } else {
                chunk.push(chars[i]);
                i += 1;
            }
        }
        // 続くひらがなは同じ文節とみなす。ただし読点のあとでは切れるようにする
        while i < chars.len() && is_hiragana(chars[i]) {
            chunk.push(chars[i]);
            i += 1;
        }
        // 行頭禁則の文字は前の塊につなげる
        while i < chars.len() && no_line_start(chars[i]) {
            chunk.push(chars[i]);
            i += 1;
        }
        chunks.push(chunk);
    }

    let width = |s: &str| s.chars().map(advance).sum::<f32>();
    let mut line = String::new();
    let mut used = 0.0;
    for chunk in chunks {
        let w = width(&chunk);
        if used + w > max_width && !line.is_empty() {
            out.push(line.trim_end().to_string());
            line = String::new();
            used = 0.0;
        }
        // 1 塊が 1 行に収まらない長さなら、文字単位で切る
        if w > max_width {
            for c in chunk.chars() {
                let a = advance(c);
                // 文字単位で切るときも禁則を守る。多少はみ出しても句読点を行頭に送らない
                let can_break = !no_line_start(c) && !line.chars().last().is_some_and(no_line_end);
                if used + a > max_width && !line.is_empty() && can_break {
                    out.push(std::mem::take(&mut line));
                    used = 0.0;
                }
                line.push(c);
                used += a;
            }
            continue;
        }
        if line.is_empty() && chunk.trim().is_empty() {
            continue;
        }
        line.push_str(&chunk);
        used += w;
    }
    out.push(line.trim_end().to_string());
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(text: &str, width: f32) -> Vec<String> {
        break_lines(text, width, &|c| if c.is_ascii() { 0.5 } else { 1.0 })
    }

    #[test]
    fn punctuation_never_starts_a_line() {
        for width in 2..12 {
            for line in lines("アプリです。次に、小さなッを含む「引用」を置きます。", width as f32)
            {
                let first = line.chars().next().unwrap();
                assert!(!no_line_start(first), "{line:?} width={width}");
            }
        }
    }

    #[test]
    fn opening_brackets_never_end_a_line() {
        for width in 2..12 {
            for line in lines("これは「引用」と（括弧）です", width as f32) {
                let last = line.chars().last().unwrap();
                assert!(!no_line_end(last), "{line:?} width={width}");
            }
        }
    }

    #[test]
    fn latin_words_stay_together_and_nothing_is_lost() {
        let text = "ここで claude-p を実行する";
        let got = lines(text, 6.0);
        assert!(got.iter().any(|l| l.contains("claude-p")), "{got:?}");
        assert_eq!(got.concat().replace(' ', ""), text.replace(' ', ""));
    }

    #[test]
    fn lines_break_between_phrases() {
        // 「レビュー」と「し、」を別の行にしない
        for width in 8..14 {
            for line in lines("手元のツールでレビューし、人が確認してから投稿する", width as f32)
            {
                assert!(!line.starts_with("し"), "{line:?} width={width}");
                assert!(!line.starts_with("て"), "{line:?} width={width}");
            }
        }
    }

    #[test]
    fn katakana_words_stay_together() {
        let got = lines("一時ディレクトリに置く", 7.0);
        assert!(got.iter().any(|l| l.contains("ディレクトリ")), "{got:?}");
    }

    #[test]
    fn very_long_words_are_split_and_newlines_kept() {
        let got = lines(&"x".repeat(30), 5.0);
        assert!(got.len() >= 3);
        assert_eq!(lines("あ\nい", 10.0), ["あ", "い"]);
    }
}

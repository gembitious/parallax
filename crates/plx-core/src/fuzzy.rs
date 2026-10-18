// 점수 상수와 정렬 방식은 fzf(https://github.com/junegunn/fzf)의 `src/algo/algo.go`를 따랐다. fzf의 저작권 고지:
//
// The MIT License (MIT)
//
// Copyright (c) 2013-2026 Junegunn Choi
//
// Permission is hereby granted, free of charge, to any person obtaining a copy
// of this software and associated documentation files (the "Software"), to deal
// in the Software without restriction, including without limitation the rights
// to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
// copies of the Software, and to permit persons to whom the Software is
// furnished to do so, subject to the following conditions:
//
// The above copyright notice and this permission notice shall be included in
// all copies or substantial portions of the Software.
//
// THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
// IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
// FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
// AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
// LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
// OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN
// THE SOFTWARE.

//! 파일 경로 퍼지 매칭(Quick Open).
//!
//! 점수는 fzf v2와 같은 방식이다. 검색어 글자를 순서대로 모두 담은 경로만 맞고, 맞은 글자마다 기본 점수에
//! 단어 경계(`/`·`_`·`-`·`.` 뒤, camelCase, 숫자 시작) 보너스를 더하고 틈은 깎는다. 연속으로 맞은 글자는 그
//! 덩어리 첫 글자의 경계 보너스를 이어받는다. 가장 좋은 정렬은 Smith-Waterman 식 동적 계획법으로 찾는다.
//!
//! 그 위에 VS Code처럼 파일 이름을 앞세운다. 낱말(공백으로 나눈 검색어 조각)이 파일 이름 안에서 맞으면 경로에서만
//! 맞는 것보다 언제나 위이고, 이름 전체·확장자를 뺀 이름·이름 앞부분과 같으면 더 위다. `/`가 든 낱말은 디렉터리까지
//! 쓴 것이라 경로 전체에서 찾는다.
//!
//! 한글: 호환 자모 자음(ㄱ~ㅎ)은 그 초성의 음절과 맞는다(`ㅍㅇ` → `파일`). IME가 조합 중일 수 있는 마지막 글자는
//! 받침이 없으면 초성·중성이 같은 음절과 맞고(`파` → `팔`), 받침이 있으면 그 받침이 겹받침으로 자라거나(`달` →
//! `닭`) 다음 음절의 초성이 될 수도 있다고 보고(`팡` → `파ㅇ`) 나은 점수를 쓴다. 그래서 `파일`을 치는 동안
//! (`ㅍ`·`파`·`팡`·`파이`·`파일`)이나 `닭`을 치는 동안(`다`·`달`·`닭`) 결과가 비지 않는다.

use std::cmp::Reverse;

const SCORE_MATCH: i32 = 16;
const SCORE_GAP_START: i32 = -3;
const SCORE_GAP_EXTENSION: i32 = -1;
const BONUS_BOUNDARY: i32 = SCORE_MATCH / 2;
const BONUS_NON_WORD: i32 = SCORE_MATCH / 2;
const BONUS_CAMEL123: i32 = BONUS_BOUNDARY + SCORE_GAP_EXTENSION;
const BONUS_CONSECUTIVE: i32 = -(SCORE_GAP_START + SCORE_GAP_EXTENSION);
const BONUS_FIRST_CHAR_MULTIPLIER: i32 = 2;
const BONUS_BOUNDARY_WHITE: i32 = BONUS_BOUNDARY + 2;
const BONUS_BOUNDARY_DELIMITER: i32 = BONUS_BOUNDARY + 1;
/// 대소문자까지 같은 글자(VS Code처럼 작은 가산점)
const BONUS_EXACT: i32 = 1;

/// 낱말이 맞은 자리의 등급. 높은 등급은 정렬 점수와 관계없이 언제나 위다.
const TIER_PATH: i32 = 0;
/// 파일 이름 안에서 맞았다
const TIER_NAME: i32 = 1;
/// 파일 이름이 낱말로 시작한다
const TIER_PREFIX: i32 = 2;
/// 확장자를 뺀 파일 이름이 낱말과 같다
const TIER_STEM: i32 = 3;
/// 파일 이름 전체가 낱말과 같다
const TIER_EXACT: i32 = 4;
/// 등급 하나의 무게. 낱말 한도(16개 × 256자)와 경로 길이 안에서 정렬 점수의 합은 이 절반보다 작아,
/// 등급 합이 큰 쪽이 언제나 위다(`Matcher::tier`로 미리 거를 수 있다).
const TIER_WEIGHT: i32 = 1 << 20;

/// 낱말 하나에서 보는 글자 수와 낱말 수 한도(점수가 등급 무게를 넘지 않게)
const MAX_TERM_CHARS: usize = 256;
const MAX_TERMS: usize = 16;

/// 동적 계획법 칸에서 "맞을 수 없음"
const NONE: i32 = i32::MIN / 2;

/// 점수를 매길 경로 글자들. ASCII 경로는 바이트를 그대로, 아니면 글자 배열을 쓴다.
pub trait Text: Copy {
    fn len(self) -> usize;
    fn at(self, i: usize) -> char;
    fn is_empty(self) -> bool {
        self.len() == 0
    }
}

impl Text for &[u8] {
    #[inline]
    fn len(self) -> usize {
        <[u8]>::len(self)
    }
    #[inline]
    fn at(self, i: usize) -> char {
        self[i] as char
    }
}

impl Text for &[char] {
    #[inline]
    fn len(self) -> usize {
        <[char]>::len(self)
    }
    #[inline]
    fn at(self, i: usize) -> char {
        self[i]
    }
}

/// 검색어 글자 하나가 경로 글자와 맞는 방법.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Needle {
    /// 대소문자를 가리지 않는 글자. `lower`는 소문자로 접은 값, `exact`는 입력한 그대로다.
    Char { lower: char, exact: char },
    /// 한글 초성(호환 자모). 그 초성으로 시작하는 음절이나 같은 자모와 맞는다.
    Initial { index: u8, jamo: char },
    /// 조합 중일 수 있는 마지막 음절. 입력한 음절이나, 초성·중성이 같고 받침이 `finals`(종성 번호 비트) 가운데
    /// 하나인 음절과 맞는다. 받침이 없으면 모든 받침(`파` → `팔`), 받침이 있으면 그 받침과 그것으로 시작하는
    /// 겹받침(`달` → `닭`, 다음 자음을 치면 겹받침이 된다).
    Syllable { typed: char, lead: u32, finals: u32 },
}

impl Needle {
    fn of(c: char) -> Needle {
        match initial_index(c) {
            Some(index) => Needle::Initial { index, jamo: c },
            None => Needle::Char {
                lower: fold(c),
                exact: c,
            },
        }
    }

    /// 경로 글자 `c`와 맞으면 `Some(입력한 그대로인가)`.
    #[inline]
    fn matches(self, c: char) -> Option<bool> {
        match self {
            Needle::Char { lower, exact } => {
                if c == exact {
                    Some(true)
                } else if fold(c) == lower {
                    Some(false)
                } else {
                    None
                }
            }
            Needle::Initial { index, jamo } => {
                if c == jamo {
                    Some(true)
                } else if initial_of(c) == Some(index) {
                    Some(false)
                } else {
                    None
                }
            }
            Needle::Syllable {
                typed,
                lead,
                finals,
            } => {
                if c == typed {
                    Some(true)
                } else if c.is_ascii() {
                    None
                } else {
                    match hangul(c) {
                        Some((l, v, t)) if l * JUNG_COUNT + v == lead && finals & (1 << t) != 0 => {
                            Some(false)
                        }
                        _ => None,
                    }
                }
            }
        }
    }
}

#[inline]
fn fold(c: char) -> char {
    if c.is_ascii() {
        c.to_ascii_lowercase()
    } else {
        c.to_lowercase().next().unwrap_or(c)
    }
}

// --- 한글 ---

const HANGUL_BASE: u32 = 0xAC00;
const HANGUL_COUNT: u32 = 11172;
const JUNG_COUNT: u32 = 21;
const JONG_COUNT: u32 = 28;
/// 초성 19개의 호환 자모(초성 순서)
const INITIALS: [char; 19] = [
    'ㄱ', 'ㄲ', 'ㄴ', 'ㄷ', 'ㄸ', 'ㄹ', 'ㅁ', 'ㅂ', 'ㅃ', 'ㅅ', 'ㅆ', 'ㅇ', 'ㅈ', 'ㅉ', 'ㅊ', 'ㅋ',
    'ㅌ', 'ㅍ', 'ㅎ',
];
/// 받침 없음(종성 0)부터 ㅎ(27)까지 모든 받침
const ALL_FINALS: u32 = (1 << JONG_COUNT) - 1;

/// 받침에 자음을 더 치면 될 수 있는 겹받침(종성 번호 비트): ㄱ → ㄳ, ㄴ → ㄵ ㄶ, ㄹ → ㄺ…ㅀ, ㅂ → ㅄ
fn growing_finals(t: u32) -> u32 {
    match t {
        1 => 1 << 3,
        4 => (1 << 5) | (1 << 6),
        8 => (9..=15).fold(0, |mask, j| mask | (1 << j)),
        17 => 1 << 18,
        _ => 0,
    }
}

/// 받침(종성 번호)이 다음 음절로 넘어갈 때 남는 받침과 넘어가는 초성(`닭`+`ㅣ` → `달기`)
const JONG_SPLIT: [(u8, u8); 28] = [
    (0, 0),  // 받침 없음(쓰지 않는다)
    (0, 0),  // ㄱ
    (0, 1),  // ㄲ
    (1, 9),  // ㄳ → ㄱ + ㅅ
    (0, 2),  // ㄴ
    (4, 12), // ㄵ → ㄴ + ㅈ
    (4, 18), // ㄶ → ㄴ + ㅎ
    (0, 3),  // ㄷ
    (0, 5),  // ㄹ
    (8, 0),  // ㄺ → ㄹ + ㄱ
    (8, 6),  // ㄻ → ㄹ + ㅁ
    (8, 7),  // ㄼ → ㄹ + ㅂ
    (8, 9),  // ㄽ → ㄹ + ㅅ
    (8, 16), // ㄾ → ㄹ + ㅌ
    (8, 17), // ㄿ → ㄹ + ㅍ
    (8, 18), // ㅀ → ㄹ + ㅎ
    (0, 6),  // ㅁ
    (0, 7),  // ㅂ
    (17, 9), // ㅄ → ㅂ + ㅅ
    (0, 9),  // ㅅ
    (0, 10), // ㅆ
    (0, 11), // ㅇ
    (0, 12), // ㅈ
    (0, 14), // ㅊ
    (0, 15), // ㅋ
    (0, 16), // ㅌ
    (0, 17), // ㅍ
    (0, 18), // ㅎ
];

/// 완성형 음절의 (초성, 중성, 종성) 번호
#[inline]
fn hangul(c: char) -> Option<(u32, u32, u32)> {
    let s = (c as u32).checked_sub(HANGUL_BASE)?;
    (s < HANGUL_COUNT).then_some((
        s / (JUNG_COUNT * JONG_COUNT),
        s % (JUNG_COUNT * JONG_COUNT) / JONG_COUNT,
        s % JONG_COUNT,
    ))
}

fn compose(l: u32, v: u32, t: u32) -> char {
    char::from_u32(HANGUL_BASE + (l * JUNG_COUNT + v) * JONG_COUNT + t).expect("한글 음절 범위")
}

/// 조합형 자모(NFD)의 초성·중성·종성 번호
fn conjoining(c: char, first: u32, count: u32) -> Option<u32> {
    let i = (c as u32).checked_sub(first)?;
    (i < count).then_some(i)
}

/// 조합형 자모(NFD로 저장된 한글: 초성 U+1100~, 중성 U+1161~, 종성 U+11A8~)를 완성형 음절로 합친다
/// (유니코드 정규화 NFC의 한글 부분. 완성형 음절 뒤의 조합형 종성도 합친다). macOS 파일 시스템이나 다른 도구가
/// NFD로 남긴 경로도 `파일`로 찾게 한다. 합친 것이 없으면 `None`, 있으면 합친 글자들과 글자마다 원래 글자의
/// 시작 위치(끝에 원래 글자 수를 하나 더: 합친 글자 `i`는 원래 `starts[i]..starts[i + 1]`).
pub fn compose_jamo(chars: &[char]) -> Option<(Vec<char>, Vec<u32>)> {
    let lead = |c| conjoining(c, 0x1100, 19);
    let vowel = |c| conjoining(c, 0x1161, JUNG_COUNT);
    let tail = |c| conjoining(c, 0x11A7, JONG_COUNT).filter(|&t| t > 0);
    // 중성·종성 자모가 없으면 합칠 것이 없다(거의 모든 경로).
    if !chars
        .iter()
        .any(|&c| vowel(c).is_some() || tail(c).is_some())
    {
        return None;
    }
    let mut out = Vec::with_capacity(chars.len());
    let mut starts = Vec::with_capacity(chars.len() + 1);
    let mut i = 0;
    while i < chars.len() {
        starts.push(i as u32);
        let mut c = chars[i];
        i += 1;
        if let Some(l) = lead(c)
            && let Some(v) = chars.get(i).and_then(|&n| vowel(n))
        {
            c = compose(l, v, 0);
            i += 1;
        }
        if let Some((l, v, 0)) = hangul(c)
            && let Some(t) = chars.get(i).and_then(|&n| tail(n))
        {
            c = compose(l, v, t);
            i += 1;
        }
        out.push(c);
    }
    if out.len() == chars.len() {
        return None;
    }
    starts.push(chars.len() as u32);
    Some((out, starts))
}

/// 초성으로 쓸 수 있는 호환 자모면 초성 번호
fn initial_index(c: char) -> Option<u8> {
    INITIALS.iter().position(|&i| i == c).map(|i| i as u8)
}

/// 경로 글자의 초성 번호: 완성형 음절이나 조합형 초성 자모(NFD로 저장된 경로)
#[inline]
fn initial_of(c: char) -> Option<u8> {
    if c.is_ascii() {
        return None;
    }
    if let Some((l, _, _)) = hangul(c) {
        return Some(l as u8);
    }
    let l = (c as u32).checked_sub(0x1100)?;
    (l < 19).then_some(l as u8)
}

// --- 글자 갈래와 경계 보너스(fzf와 같다) ---

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Class {
    White,
    NonWord,
    Delimiter,
    Lower,
    Upper,
    Letter,
    Number,
}

#[inline]
fn class(c: char) -> Class {
    if c.is_ascii() {
        match c {
            'a'..='z' => Class::Lower,
            'A'..='Z' => Class::Upper,
            '0'..='9' => Class::Number,
            '/' => Class::Delimiter,
            ' ' | '\t' => Class::White,
            _ => Class::NonWord,
        }
    } else if c.is_whitespace() {
        Class::White
    } else if c.is_lowercase() {
        Class::Lower
    } else if c.is_uppercase() {
        Class::Upper
    } else if c.is_numeric() {
        Class::Number
    } else if c.is_alphabetic() {
        Class::Letter
    } else {
        Class::NonWord
    }
}

#[inline]
fn bonus_for(prev: Class, class: Class) -> i32 {
    if class > Class::NonWord {
        match prev {
            Class::White => return BONUS_BOUNDARY_WHITE,
            Class::Delimiter => return BONUS_BOUNDARY_DELIMITER,
            Class::NonWord => return BONUS_BOUNDARY,
            _ => {}
        }
    }
    if prev == Class::Lower && class == Class::Upper
        || prev != Class::Number && class == Class::Number
    {
        return BONUS_CAMEL123;
    }
    match class {
        Class::NonWord | Class::Delimiter => BONUS_NON_WORD,
        Class::White => BONUS_BOUNDARY_WHITE,
        _ => 0,
    }
}

// --- 검색어 ---

/// 검색어 낱말 하나.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Term {
    /// 입력한 글자(좁히기 판단용)
    text: String,
    /// 맞추는 방법. 보통 하나이고, 조합 중일 수 있는 받침 있는 마지막 음절이면 둘이다.
    variants: Vec<Vec<Needle>>,
    /// `/`가 있으면 경로 전체에서만 찾는다
    path: bool,
}

/// Quick Open 검색어: 공백으로 나눈 낱말(모두 맞아야 한다)과 끝의 `:줄`(`:줄:열`).
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Query {
    terms: Vec<Term>,
    /// `파일:줄`의 줄(1부터)
    pub line: Option<u32>,
}

impl Query {
    /// `raw`를 낱말로 나눈다. `\`는 `/`로 본다(Windows 경로를 붙여 넣어도 된다). NFD 한글은 합친다
    /// (NFD로 저장된 파일 이름을 복사해 붙여 넣어도 된다).
    pub fn parse(raw: &str) -> Query {
        let mut text = raw.trim_start().replace('\\', "/");
        if !text.is_ascii()
            && let Some((composed, _)) = compose_jamo(&text.chars().collect::<Vec<char>>())
        {
            text = composed.into_iter().collect();
        }
        // 공백으로 끝나면 마지막 낱말의 조합은 끝났다.
        let composing = !text.ends_with(char::is_whitespace);
        let (body, line) = split_line(text.trim_end());
        let words: Vec<&str> = body.split_whitespace().take(MAX_TERMS).collect();
        let count = words.len();
        let terms = words
            .into_iter()
            .enumerate()
            .map(|(i, word)| term(word, composing && line.is_none() && i + 1 == count))
            .collect();
        Query { terms, line }
    }

    pub fn is_empty(&self) -> bool {
        self.terms.is_empty()
    }

    /// 이 검색어에 맞는 경로가 모두 `previous`에도 맞는가(앞 결과 안에서만 찾아도 되는가).
    /// 낱말마다 앞 낱말을 이어 친 것이면 그렇다(퍼지 일치는 글자를 더할수록 좁아진다). 글자가 같으면 맞추는
    /// 방법도 같아야 한다: 조합이 끝난 `파 `를 지워 `파`로 돌아오면 `팔`도 맞으므로 앞 결과보다 넓다.
    pub fn narrows(&self, previous: &Query) -> bool {
        self.terms.len() >= previous.terms.len()
            && self.terms.iter().zip(&previous.terms).all(|(new, old)| {
                new.text.starts_with(&old.text)
                    && (new.text.len() > old.text.len() || new.variants == old.variants)
            })
    }
}

/// 끝의 `:줄`·`:줄:열`(열은 버린다)을 뗀다. 치는 중인 `:` 하나도 뗀다.
fn split_line(text: &str) -> (&str, Option<u32>) {
    let text = text.strip_suffix(':').unwrap_or(text);
    let Some((rest, last)) = split_number(text) else {
        return (text, None);
    };
    let (body, line) = match split_number(rest) {
        Some((body, line)) => (body, line),
        None => (rest, last),
    };
    if body.trim().is_empty() || line == 0 {
        (text, None)
    } else {
        (body, Some(line))
    }
}

/// `앞:숫자`면 (앞, 숫자)
fn split_number(text: &str) -> Option<(&str, u32)> {
    let (rest, digits) = text.rsplit_once(':')?;
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    Some((rest, digits.parse().ok()?))
}

fn term(word: &str, composing: bool) -> Term {
    let chars: Vec<char> = word.chars().take(MAX_TERM_CHARS).collect();
    let mut needles: Vec<Needle> = chars.iter().map(|&c| Needle::of(c)).collect();
    let mut variants = Vec::with_capacity(2);
    match chars
        .last()
        .copied()
        .and_then(|c| hangul(c).map(|h| (c, h)))
    {
        Some((typed, (l, v, 0))) if composing => {
            *needles.last_mut().expect("글자가 있다") = Needle::Syllable {
                typed,
                lead: l * JUNG_COUNT + v,
                finals: ALL_FINALS,
            };
            variants.push(needles);
        }
        Some((typed, (l, v, t))) if composing => {
            let (rest, moved) = JONG_SPLIT[t as usize];
            let mut split = needles.clone();
            split.pop();
            let syllable = compose(l, v, rest as u32);
            split.push(Needle::Char {
                lower: syllable,
                exact: syllable,
            });
            split.push(Needle::Initial {
                index: moved,
                jamo: INITIALS[moved as usize],
            });
            *needles.last_mut().expect("글자가 있다") = Needle::Syllable {
                typed,
                lead: l * JUNG_COUNT + v,
                finals: (1 << t) | growing_finals(t),
            };
            variants.push(needles);
            variants.push(split);
        }
        _ => variants.push(needles),
    }
    Term {
        text: word.to_owned(),
        variants,
        path: word.contains('/'),
    }
}

// --- 점수 ---

/// 낱말 하나(변형 하나)가 경로에서 맞은 자리: 등급과 점수를 매길 구간, 이름 앞부분 일치면 그 시작.
struct Place {
    tier: i32,
    window: (usize, usize),
    prefix: Option<usize>,
}

/// 낱말이 맞는 자리. 파일 이름 안에서 맞으면 그쪽(등급이 높다), 아니면 경로 전체.
fn locate<T: Text>(needles: &[Needle], path_only: bool, hay: T, base: usize) -> Option<Place> {
    // 경로 전체에도 안 맞으면 볼 것 없다(가장 흔한 경우라 먼저).
    let whole = window(needles, hay, 0)?;
    if !path_only
        && base < hay.len()
        && let Some(name) = window(needles, hay, base)
    {
        let (prefix, tier) = match name_prefix(needles, hay, base) {
            Some((start, tier)) => (Some(start), tier),
            None => (None, TIER_NAME),
        };
        return Some(Place {
            tier,
            window: name,
            prefix,
        });
    }
    Some(Place {
        tier: TIER_PATH,
        window: whole,
        prefix: None,
    })
}

/// 점수 계산 작업 공간. 경로마다 새로 할당하지 않게 다시 쓴다(스레드마다 하나).
#[derive(Default)]
pub struct Matcher {
    bonus: Vec<i32>,
    // 점수만 셀 때 쓰는 두 줄(앞 글자 줄, 지금 줄)
    prev: Vec<i32>,
    prev_chunk: Vec<i32>,
    cur: Vec<i32>,
    cur_chunk: Vec<i32>,
    // 글자마다 놓일 수 있는 열의 범위
    lo: Vec<usize>,
    hi: Vec<usize>,
    // 위치까지 셀 때 쓰는 전체 표
    score: Vec<i32>,
    chunk: Vec<i32>,
    from: Vec<u32>,
    positions: Vec<u32>,
}

impl Matcher {
    pub fn new() -> Matcher {
        Matcher::default()
    }

    /// 등급만(정렬 점수 없이) 매긴다. 맞지 않으면 `None`. 등급이 다르면 `score`도 언제나 같은 쪽이 크다
    /// (등급 무게가 정렬 점수보다 크다). 앞 몇 개만 볼 때 들 수 없는 경로를 점수 계산 전에 거르는 데 쓴다.
    pub fn tier<T: Text>(&mut self, query: &Query, path: T, base: usize) -> Option<i32> {
        let mut total = 0;
        for term in &query.terms {
            let best = term
                .variants
                .iter()
                .filter_map(|needles| {
                    locate(needles, term.path, path, base).map(|place| place.tier)
                })
                .max()?;
            total += best * TIER_WEIGHT;
        }
        Some(total)
    }

    /// 경로 점수(클수록 위). 맞지 않으면 `None`. `base`는 파일 이름이 시작하는 글자 위치다.
    /// `positions`가 있으면 맞은 글자 위치(글자 단위, 오름차순)를 채운다.
    pub fn score<T: Text>(
        &mut self,
        query: &Query,
        path: T,
        base: usize,
        mut positions: Option<&mut Vec<u32>>,
    ) -> Option<i32> {
        if let Some(positions) = positions.as_deref_mut() {
            positions.clear();
        }
        let want = positions.is_some();
        let mut total = 0;
        for term in &query.terms {
            let mut best: Option<(i32, Vec<u32>)> = None;
            for needles in &term.variants {
                let Some(place) = locate(needles, term.path, path, base) else {
                    continue;
                };
                let aligned = if want {
                    self.align(needles, path, place.window, true)
                } else {
                    self.align_score(needles, path, place.window)
                };
                let Some(aligned) = aligned else {
                    continue;
                };
                let score = place.tier * TIER_WEIGHT + aligned;
                if best.as_ref().is_some_and(|(b, _)| score <= *b) {
                    continue;
                }
                let found = match (want, place.prefix) {
                    (false, _) => Vec::new(),
                    // 이름 앞부분 일치는 그 앞부분을 칠한다
                    (true, Some(start)) => {
                        (start..start + needles.len()).map(|p| p as u32).collect()
                    }
                    (true, None) => self.positions.clone(),
                };
                best = Some((score, found));
            }
            let (score, found) = best?;
            total += score;
            if let Some(positions) = positions.as_deref_mut() {
                positions.extend(found);
            }
        }
        if let Some(positions) = positions {
            positions.sort_unstable();
            positions.dedup();
        }
        Some(total)
    }

    /// 구간 글자들의 경계 보너스
    fn prepare_bonus<T: Text>(&mut self, hay: T, first: usize, last: usize) {
        self.bonus.clear();
        let mut prev = if first == 0 {
            Class::Delimiter
        } else {
            class(hay.at(first - 1))
        };
        for j in first..=last {
            let c = class(hay.at(j));
            self.bonus.push(bonus_for(prev, c));
            prev = c;
        }
    }

    /// `align`과 같은 점수를 두 줄만 써서 센다(위치 없이). 검색마다 맞은 경로 모두에 쓰는 쪽이다.
    /// 글자마다 놓일 수 있는 열(앞에서 욕심껏 맞춘 자리 ~ 뒤에서 욕심껏 맞춘 자리)만 본다. 그 밖의 칸은 끝까지
    /// 이어지는 정렬에 들 수 없어 점수가 같다.
    fn align_score<T: Text>(
        &mut self,
        needles: &[Needle],
        hay: T,
        (first, last): (usize, usize),
    ) -> Option<i32> {
        let m = needles.len();
        let n = last - first + 1;
        self.prepare_bonus(hay, first, last);
        // 구간은 첫 글자가 맞는 곳에서 시작해 끝 글자가 맞는 곳에서 끝난다(`window`).
        self.lo.clear();
        self.lo.push(0);
        let mut j = 0;
        for needle in &needles[1..] {
            j += 1;
            while needle.matches(hay.at(first + j)).is_none() {
                j += 1;
            }
            self.lo.push(j);
        }
        self.hi.clear();
        self.hi.resize(m, n - 1);
        let mut j = n - 1;
        for i in (0..m - 1).rev() {
            j -= 1;
            while needles[i].matches(hay.at(first + j)).is_none() {
                j -= 1;
            }
            self.hi[i] = j;
        }
        for row in [&mut self.prev, &mut self.cur] {
            row.clear();
            row.resize(n, NONE);
        }
        for row in [&mut self.prev_chunk, &mut self.cur_chunk] {
            row.clear();
            row.resize(n, 0);
        }
        for j in 0..=self.hi[0] {
            self.prev[j] = match needles[0].matches(hay.at(first + j)) {
                Some(exact) => {
                    self.prev_chunk[j] = self.bonus[j];
                    SCORE_MATCH
                        + self.bonus[j] * BONUS_FIRST_CHAR_MULTIPLIER
                        + if exact { BONUS_EXACT } else { 0 }
                }
                None => NONE,
            };
        }
        for (i, needle) in needles.iter().enumerate().skip(1) {
            let (plo, phi) = (self.lo[i - 1], self.hi[i - 1]);
            let (lo, hi) = (self.lo[i], self.hi[i]);
            let (prev, prev_chunk) = (&self.prev, &self.prev_chunk);
            let (cur, cur_chunk) = (&mut self.cur, &mut self.cur_chunk);
            // 앞 줄은 [plo, phi]만 뜻이 있다. 틈 값은 plo부터 쌓아 lo에 이른다.
            let mut gap = NONE;
            for j in plo + 1..=hi {
                if j >= plo + 2 {
                    let opened = if j - 2 <= phi { prev[j - 2] } else { NONE };
                    let extended = if gap > NONE {
                        gap + SCORE_GAP_EXTENSION
                    } else {
                        NONE
                    };
                    gap = if opened > NONE && opened + SCORE_GAP_START >= extended {
                        opened + SCORE_GAP_START
                    } else {
                        extended
                    };
                }
                if j < lo {
                    continue;
                }
                cur[j] = NONE;
                let Some(exact) = needle.matches(hay.at(first + j)) else {
                    continue;
                };
                let b = self.bonus[j];
                let mut best = NONE;
                let mut chunk = 0;
                let diag = if j - 1 <= phi { prev[j - 1] } else { NONE };
                if diag > NONE {
                    let start = prev_chunk[j - 1];
                    let (bonus, chunk_bonus) = if b >= BONUS_BOUNDARY && b > start {
                        (b, b)
                    } else {
                        (b.max(start).max(BONUS_CONSECUTIVE), start)
                    };
                    best = diag + bonus;
                    chunk = chunk_bonus;
                }
                if gap > NONE && gap + b > best {
                    best = gap + b;
                    chunk = b;
                }
                if best > NONE {
                    cur[j] = best + SCORE_MATCH + if exact { BONUS_EXACT } else { 0 };
                    cur_chunk[j] = chunk;
                }
            }
            std::mem::swap(&mut self.prev, &mut self.cur);
            std::mem::swap(&mut self.prev_chunk, &mut self.cur_chunk);
        }
        let (lo, hi) = (self.lo[m - 1], self.hi[m - 1]);
        self.prev[lo..=hi]
            .iter()
            .copied()
            .filter(|&s| s > NONE)
            .max()
    }

    /// `window` 안에서 `needles`를 순서대로 맞춘 가장 좋은 점수(fzf v2). `want`면 위치를 `self.positions`에 둔다.
    fn align<T: Text>(
        &mut self,
        needles: &[Needle],
        hay: T,
        (first, last): (usize, usize),
        want: bool,
    ) -> Option<i32> {
        let m = needles.len();
        let n = last - first + 1;
        self.prepare_bonus(hay, first, last);
        let cells = m * n;
        self.score.clear();
        self.score.resize(cells, NONE);
        self.chunk.clear();
        self.chunk.resize(cells, 0);
        self.from.clear();
        self.from.resize(cells, u32::MAX);

        for j in 0..n {
            if let Some(exact) = needles[0].matches(hay.at(first + j)) {
                self.score[j] = SCORE_MATCH
                    + self.bonus[j] * BONUS_FIRST_CHAR_MULTIPLIER
                    + if exact { BONUS_EXACT } else { 0 };
                self.chunk[j] = self.bonus[j];
            }
        }
        for (i, needle) in needles.iter().enumerate().skip(1) {
            let row = i * n;
            let up = row - n;
            // gap: j-2 이전 열에서 끝난 앞 글자 + 틈 벌점의 최댓값과 그 열
            let mut gap = NONE;
            let mut gap_from = u32::MAX;
            for j in i..n {
                if j >= 2 {
                    let opened = self.score[up + j - 2];
                    let extended = if gap > NONE {
                        gap + SCORE_GAP_EXTENSION
                    } else {
                        NONE
                    };
                    if opened > NONE && opened + SCORE_GAP_START >= extended {
                        gap = opened + SCORE_GAP_START;
                        gap_from = (j - 2) as u32;
                    } else {
                        gap = extended;
                    }
                }
                let Some(exact) = needle.matches(hay.at(first + j)) else {
                    continue;
                };
                let b = self.bonus[j];
                let mut best = NONE;
                let mut chunk = 0;
                let mut from = u32::MAX;
                let diag = self.score[up + j - 1];
                if diag > NONE {
                    // 연속: 덩어리 첫 글자의 보너스를 이어받는다. 더 큰 경계에서는 새 덩어리를 시작한다.
                    let start = self.chunk[up + j - 1];
                    let (bonus, chunk_bonus) = if b >= BONUS_BOUNDARY && b > start {
                        (b, b)
                    } else {
                        (b.max(start).max(BONUS_CONSECUTIVE), start)
                    };
                    best = diag + bonus;
                    chunk = chunk_bonus;
                    from = (j - 1) as u32;
                }
                if gap > NONE && gap + b > best {
                    best = gap + b;
                    chunk = b;
                    from = gap_from;
                }
                if best > NONE {
                    self.score[row + j] = best + SCORE_MATCH + if exact { BONUS_EXACT } else { 0 };
                    self.chunk[row + j] = chunk;
                    self.from[row + j] = from;
                }
            }
        }

        let row = (m - 1) * n;
        // 같은 점수면 앞쪽
        let end = (0..n)
            .filter(|&j| self.score[row + j] > NONE)
            .max_by_key(|&j| (self.score[row + j], Reverse(j)))?;
        if want {
            self.positions.clear();
            let mut j = end;
            for i in (0..m).rev() {
                self.positions.push((first + j) as u32);
                if i > 0 {
                    j = self.from[i * n + j] as usize;
                }
            }
            self.positions.reverse();
        }
        Some(self.score[row + end])
    }
}

/// `hay[lo..]`에서 맞을 수 있는 구간: 첫 글자가 처음 맞는 곳부터 끝 글자가 마지막으로 맞는 곳까지.
/// 순서대로 다 맞출 수 없으면 `None`.
fn window<T: Text>(needles: &[Needle], hay: T, lo: usize) -> Option<(usize, usize)> {
    let len = hay.len();
    let first = (lo..len).find(|&j| needles[0].matches(hay.at(j)).is_some())?;
    let mut next = first + 1;
    for needle in &needles[1..] {
        next = (next..len).find(|&j| needle.matches(hay.at(j)).is_some())? + 1;
    }
    let tail = needles[needles.len() - 1];
    let last = (next - 1..len)
        .rev()
        .find(|&j| tail.matches(hay.at(j)).is_some())
        .unwrap_or(next - 1);
    Some((first, last))
}

/// 파일 이름이 낱말로 시작하면 (시작 위치, 등급). 점파일(`.gitignore`)은 점을 빼고도 견준다.
fn name_prefix<T: Text>(needles: &[Needle], hay: T, base: usize) -> Option<(usize, i32)> {
    let len = hay.len();
    let starts = |at: usize| {
        at + needles.len() <= len
            && needles
                .iter()
                .enumerate()
                .all(|(i, needle)| needle.matches(hay.at(at + i)).is_some())
    };
    let start = if starts(base) {
        base
    } else if hay.at(base) == '.' && starts(base + 1) {
        base + 1
    } else {
        return None;
    };
    let end = start + needles.len();
    let tier = if end == len {
        TIER_EXACT
    } else if hay.at(end) == '.' && (end + 1..len).all(|j| hay.at(j) != '.') {
        TIER_STEM
    } else {
        TIER_PREFIX
    };
    Some((start, tier))
}

/// 경로에서 파일 이름이 시작하는 글자 위치
pub fn base_of<T: Text>(path: T) -> usize {
    (0..path.len())
        .rev()
        .find(|&i| path.at(i) == '/')
        .map_or(0, |i| i + 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chars(path: &str) -> Vec<char> {
        path.chars().collect()
    }

    fn score_of(query: &str, path: &str) -> Option<i32> {
        let q = Query::parse(query);
        let c = chars(path);
        let mut m = Matcher::new();
        let base = base_of(c.as_slice());
        if path.is_ascii() {
            let a = m.score(&q, path.as_bytes(), base, None);
            // ASCII 바이트와 글자 배열은 같은 점수다.
            assert_eq!(a, m.score(&q, c.as_slice(), base, None), "{query} / {path}");
            a
        } else {
            m.score(&q, c.as_slice(), base, None)
        }
    }

    fn positions_of(query: &str, path: &str) -> Vec<u32> {
        let q = Query::parse(query);
        let c = chars(path);
        let mut positions = Vec::new();
        Matcher::new()
            .score(
                &q,
                c.as_slice(),
                base_of(c.as_slice()),
                Some(&mut positions),
            )
            .unwrap_or_else(|| panic!("{query} / {path}"));
        positions
    }

    /// 점수 순(같으면 짧은 경로 먼저)
    fn rank<'a>(query: &str, paths: &[&'a str]) -> Vec<&'a str> {
        let mut scored: Vec<(i32, &str)> = paths
            .iter()
            .filter_map(|p| score_of(query, p).map(|s| (s, *p)))
            .collect();
        scored.sort_by_key(|&(s, p)| (Reverse(s), p.chars().count(), p));
        scored.into_iter().map(|(_, p)| p).collect()
    }

    #[test]
    fn subsequence_in_order() {
        assert!(score_of("gv", "src/graph/GraphView.tsx").is_some());
        assert!(
            score_of("vg", "src/graph/GraphView.tsx").is_none(),
            "순서가 다르다"
        );
        assert!(score_of("xyz", "src/graph/GraphView.tsx").is_none());
        assert!(
            score_of("GRAPH", "src/graph/draw.ts").is_some(),
            "대소문자 무시"
        );
        assert!(score_of("draw.tsx", "src/graph/draw.ts").is_none());
        assert!(
            score_of("", "anything").is_some(),
            "빈 검색어는 모두 맞는다"
        );
    }

    #[test]
    fn all_terms_must_match() {
        assert!(score_of("graph view", "src/graph/GraphView.tsx").is_some());
        assert!(score_of("graph nope", "src/graph/GraphView.tsx").is_none());
        // 낱말은 서로 겹쳐도 된다
        assert!(score_of("graph graph", "src/graph/draw.ts").is_some());
    }

    #[test]
    fn file_name_beats_directories() {
        let paths = [
            "src/graph/draw.ts",
            "src/graph/rows.ts",
            "crates/plx-core/src/graph.rs",
            "crates/plx-core/tests/graph.rs",
            "src/graph/GraphView.tsx",
            "docs/paragraph.md",
        ];
        assert_eq!(
            rank("graph", &paths),
            [
                // 확장자를 뺀 이름이 같다(짧은 경로 먼저)
                "crates/plx-core/src/graph.rs",
                "crates/plx-core/tests/graph.rs",
                // 이름이 낱말로 시작
                "src/graph/GraphView.tsx",
                // 이름 안에서
                "docs/paragraph.md",
                // 디렉터리에서만
                "src/graph/draw.ts",
                "src/graph/rows.ts",
            ]
        );
    }

    #[test]
    fn exact_name_and_dotfiles() {
        let paths = ["Makefile", "src/Makefile.toml", "docs/makefiles.md"];
        assert_eq!(rank("makefile", &paths), paths);
        assert_eq!(
            rank(
                "gitignore",
                &[".github/ignore.yml", "src/.gitignore", ".gitignore"]
            ),
            [".gitignore", "src/.gitignore", ".github/ignore.yml"]
        );
    }

    #[test]
    fn boundaries_beat_scattered_matches() {
        // 단어 머리글자(camelCase, `_`, `/`)가 흩어진 글자보다 위
        assert_eq!(
            rank("gv", &["src/gravel.ts", "src/GraphView.tsx"]),
            ["src/GraphView.tsx", "src/gravel.ts"]
        );
        assert_eq!(
            rank("bv", &["src/blame/abvx.ts", "src/blame/BlameView.tsx"]),
            ["src/blame/BlameView.tsx", "src/blame/abvx.ts"]
        );
        // 연속으로 맞은 덩어리가 흩어진 글자보다 위
        assert_eq!(
            rank("draw", &["src/dxrxaxw.ts", "src/redraw.ts"]),
            ["src/redraw.ts", "src/dxrxaxw.ts"]
        );
    }

    #[test]
    fn slash_terms_match_whole_path() {
        assert_eq!(
            rank(
                "graph/draw",
                &["src/graph/draw.ts", "src/graphdraw.ts", "src/draw.ts"]
            ),
            ["src/graph/draw.ts"]
        );
        assert_eq!(
            rank(
                "src/view",
                &["src/view.ts", "lib/src-view.ts", "src/blame/BlameView.tsx"]
            ),
            ["src/view.ts", "src/blame/BlameView.tsx"]
        );
    }

    #[test]
    fn positions_follow_the_best_alignment() {
        assert_eq!(positions_of("gv", "src/graph/GraphView.tsx"), [10, 15]);
        // 이름 앞부분 일치는 그 앞부분을 칠한다
        assert_eq!(
            positions_of("graph", "src/graph/graph.rs"),
            [10, 11, 12, 13, 14]
        );
        // 경로 일치
        assert_eq!(positions_of("sgd", "src/graph/draw.ts"), [0, 4, 10]);
        // 낱말 여럿은 합친다
        assert_eq!(
            positions_of("rs graph", "src/graph.rs"),
            [4, 5, 6, 7, 8, 10, 11]
        );
    }

    #[test]
    fn exact_case_breaks_ties() {
        assert!(score_of("View", "src/View.ts") > score_of("View", "src/view.ts"));
        assert!(score_of("view", "src/view.ts") > score_of("view", "src/View.ts"));
    }

    #[test]
    fn korean_initials_and_syllables() {
        let path = "소스/파일 이름.rs";
        assert!(score_of("파일", path).is_some());
        assert!(score_of("ㅍㅇ", path).is_some(), "초성");
        assert!(score_of("ㅅㅅ ㅇㄹ", path).is_some());
        assert!(score_of("ㅍ이", path).is_some(), "초성과 음절 섞기");
        assert!(score_of("ㄱㅇ", path).is_none());
        assert_eq!(positions_of("ㅍㅇ", path), [3, 4]);
        // 파일 이름 앞부분(초성으로도)
        assert_eq!(
            rank("ㅍㅇ", &["파일/소스.rs", "소스/파일 이름.rs"]),
            ["소스/파일 이름.rs", "파일/소스.rs"]
        );
    }

    #[test]
    fn composing_syllables_keep_matching() {
        let path = "소스/파일 이름.rs";
        // 두벌식으로 `파일`을 치는 동안 IME가 보이는 글자들
        for typing in ["ㅍ", "파", "팡", "파이", "파일"] {
            assert!(score_of(typing, path).is_some(), "{typing}");
        }
        // 조합이 끝난(공백이 뒤따른) 음절은 그대로만 맞는다
        assert!(score_of("팡 ", path).is_none());
        assert!(score_of("파 ", "소스/팔.rs").is_none());
        assert!(score_of("파", "소스/팔.rs").is_some());
        // 겹받침: `닭`은 `달ㄱ`일 수 있다
        assert!(score_of("닭", "달걀.txt").is_some());
        assert!(score_of("닭 ", "달걀.txt").is_none());
        // 반대로 `닭`을 치는 중의 `달`(다음 자음이 겹받침이 된다)
        for typing in ["ㄷ", "다", "달", "닭"] {
            assert!(score_of(typing, "닭.txt").is_some(), "{typing}");
        }
        assert!(score_of("달 ", "닭.txt").is_none(), "조합이 끝난 달");
        assert!(score_of("각", "갃.txt").is_some(), "ㄱ → ㄳ");
        assert!(score_of("간", "갅.txt").is_some(), "ㄴ → ㄵ");
        assert!(score_of("갑", "값.txt").is_some(), "ㅂ → ㅄ");
        assert!(
            score_of("감", "갉.txt").is_none(),
            "ㅁ은 겹받침이 되지 않는다"
        );
        // 그대로 맞는 쪽이 위
        assert!(score_of("파", "파.rs") > score_of("파", "팔.rs"));
    }

    #[test]
    fn nfd_paths_match_initials() {
        // macOS에서 정규화 없이 들어온 경로(조합형 자모)도 초성으로는 찾는다
        let nfd = "\u{1111}\u{1161}\u{110B}\u{1175}\u{11AF}.rs"; // 파일.rs
        assert!(score_of("ㅍㅇ", nfd).is_some());
    }

    #[test]
    fn composes_conjoining_jamo() {
        let nfd = chars("a/\u{1111}\u{1161}\u{110B}\u{1175}\u{11AF}.rs"); // a/파일.rs
        let (composed, starts) = compose_jamo(&nfd).unwrap();
        assert_eq!(composed, chars("a/파일.rs"));
        assert_eq!(starts, [0, 1, 2, 4, 7, 8, 9, 10]);
        // 완성형 음절 + 조합형 종성, 홀로 선 자모(합칠 짝이 없다)
        assert_eq!(compose_jamo(&chars("파\u{11AF}")).unwrap().0, chars("팔"));
        assert_eq!(
            compose_jamo(&chars("\u{1100}x\u{1161}\u{11AF}")),
            None,
            "합친 것이 없다"
        );
        assert_eq!(
            compose_jamo(&chars("팔\u{11AF}")),
            None,
            "받침 위에 받침은 없다"
        );
        assert_eq!(compose_jamo(&chars("src/파일.rs")), None);
        // 붙여 넣은 NFD 검색어도 합친다
        assert_eq!(
            Query::parse("\u{1111}\u{1161}\u{110B}\u{1175}\u{11AF}"),
            Query::parse("파일")
        );
    }

    #[test]
    fn parses_line_suffix() {
        let q = Query::parse("graph.rs:120");
        assert_eq!(q.line, Some(120));
        assert_eq!(q.terms.len(), 1);
        assert_eq!(q.terms[0].text, "graph.rs");
        assert_eq!(Query::parse("graph.rs:120:4").line, Some(120));
        assert_eq!(
            Query::parse("graph.rs:").terms[0].text,
            "graph.rs",
            "치는 중인 :"
        );
        assert_eq!(Query::parse("graph.rs:").line, None);
        assert_eq!(Query::parse(":12").line, None, "파일 없는 줄");
        assert_eq!(Query::parse(":12").terms[0].text, ":12");
        assert_eq!(Query::parse("a:0").line, None);
        assert_eq!(Query::parse("C:/x/y.rs").terms[0].text, "C:/x/y.rs");
        assert_eq!(Query::parse("src\\graph.rs").terms[0].text, "src/graph.rs");
        assert!(Query::parse("src\\graph.rs").terms[0].path);
        assert!(Query::parse("   ").is_empty());
        assert_eq!(Query::parse("a b  c").terms.len(), 3);
    }

    #[test]
    fn narrowing_rule() {
        let q = Query::parse;
        assert!(q("gra").narrows(&q("gr")));
        assert!(q("gr v").narrows(&q("gr")));
        assert!(q("gr:12").narrows(&q("gr:1")));
        assert!(q("gr:1").narrows(&q("gr:")));
        assert!(!q("gr").narrows(&q("gra")));
        assert!(!q("g").narrows(&q("g v")));
        assert!(!q("파이").narrows(&q("팡")));
        assert!(q("x").narrows(&q("")));
        // 지워서 같은 글자로 돌아와도 조합 중이면 더 넓다
        assert!(!q("파").narrows(&q("파 ")));
        assert!(!q("파:").narrows(&q("파:3")));
        assert!(q("파일").narrows(&q("파")));
    }

    /// 좁혀도 되는 검색어 쌍이면 새 검색어에 맞는 경로는 모두 앞 검색어에도 맞는다(무작위 대조).
    #[test]
    fn narrowing_never_loses_matches() {
        let alphabet: Vec<char> = "abAB/._ 파팔팡이일ㅍㅇ".chars().collect();
        let mut seed = 0x2545_f491_u64;
        let mut next = |n: usize| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            (seed % n as u64) as usize
        };
        let mut m = Matcher::new();
        for _ in 0..3000 {
            let path: String = (0..1 + next(12))
                .map(|_| alphabet[next(alphabet.len())])
                .collect();
            let path = path.trim().replace(' ', "_");
            if path.is_empty() {
                continue;
            }
            let old: String = (0..next(4))
                .map(|_| alphabet[next(alphabet.len())])
                .collect();
            // 이어 치기, 지우기(앞부분만 남기기), 끝 공백 넣고 빼기
            let new: String = match next(3) {
                0 => format!(
                    "{old}{}",
                    (0..1 + next(3))
                        .map(|_| alphabet[next(alphabet.len())])
                        .collect::<String>()
                ),
                1 => old.chars().take(next(old.chars().count() + 1)).collect(),
                _ => match old.strip_suffix(' ') {
                    Some(trimmed) => trimmed.to_owned(),
                    None => format!("{old} "),
                },
            };
            let (old_q, new_q) = (Query::parse(&old), Query::parse(&new));
            if !new_q.narrows(&old_q) {
                continue;
            }
            let c = chars(&path);
            let base = base_of(c.as_slice());
            if m.score(&new_q, c.as_slice(), base, None).is_some() {
                assert!(
                    m.score(&old_q, c.as_slice(), base, None).is_some(),
                    "{new:?}가 맞는 {path:?}에 {old:?}가 안 맞는다"
                );
            }
        }
    }

    /// 돌려준 위치는 검색어 글자와 맞고, 낱말마다 순서대로다(무작위 대조).
    #[test]
    fn positions_are_valid() {
        let alphabet: Vec<char> = "abcAB/._-1파일ㅍ".chars().collect();
        let mut seed = 0x9e37_79b9_u64;
        let mut next = |n: usize| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            (seed % n as u64) as usize
        };
        let mut m = Matcher::new();
        let mut checked = 0;
        for _ in 0..5000 {
            let path: String = (0..1 + next(20))
                .map(|_| alphabet[next(alphabet.len())])
                .collect();
            let word: String = (0..1 + next(4))
                .map(|_| alphabet[next(alphabet.len())])
                .collect();
            let q = Query::parse(&format!("{word} "));
            let Some(term) = q.terms.first() else {
                continue;
            };
            let needles = &term.variants[0];
            let c = chars(&path);
            let base = base_of(c.as_slice());
            let mut positions = Vec::new();
            let with = m.score(&q, c.as_slice(), base, Some(&mut positions));
            let without = m.score(&q, c.as_slice(), base, None);
            assert_eq!(with, without, "{word:?} / {path:?}");
            if with.is_none() {
                continue;
            }
            checked += 1;
            assert_eq!(
                positions.len(),
                needles.len(),
                "{word:?} / {path:?}: {positions:?}"
            );
            for (needle, &p) in needles.iter().zip(&positions) {
                assert!(
                    needle.matches(c[p as usize]).is_some(),
                    "{word:?} / {path:?}: {positions:?}"
                );
            }
            assert!(positions.windows(2).all(|w| w[0] < w[1]));
        }
        assert!(checked > 500, "맞은 경우가 너무 적다: {checked}");
    }

    /// 등급 합이 큰 경로는 점수도 크다. 그래서 앞 몇 개만 고를 때 등급으로 먼저 걸러도 결과가 같다(무작위 대조).
    #[test]
    fn tiers_dominate_scores() {
        let alphabet: Vec<char> = "abcAB/._-파일ㅍ".chars().collect();
        let mut seed = 0x51ed_270b_u64;
        let mut next = |n: usize| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            (seed % n as u64) as usize
        };
        let mut m = Matcher::new();
        for _ in 0..300 {
            let query: String = (0..1 + next(5))
                .map(|_| alphabet[next(alphabet.len())])
                .collect();
            let q = Query::parse(&query);
            let mut scored = Vec::new();
            for _ in 0..40 {
                let path: String = (0..1 + next(24))
                    .map(|_| alphabet[next(alphabet.len())])
                    .collect();
                let c = chars(&path);
                let base = base_of(c.as_slice());
                let tier = m.tier(&q, c.as_slice(), base);
                let score = m.score(&q, c.as_slice(), base, None);
                assert_eq!(tier.is_some(), score.is_some(), "{query:?} / {path:?}");
                if let (Some(tier), Some(score)) = (tier, score) {
                    scored.push((tier, score, path));
                }
            }
            for (ta, sa, pa) in &scored {
                for (tb, sb, pb) in &scored {
                    if ta > tb {
                        assert!(
                            sa > sb,
                            "{query:?}: {pa:?}({ta}, {sa}) / {pb:?}({tb}, {sb})"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn hangul_tables() {
        assert_eq!(hangul('가'), Some((0, 0, 0)));
        assert_eq!(hangul('힣'), Some((18, 20, 27)));
        assert_eq!(hangul('a'), None);
        assert_eq!(compose(17, 0, 0), '파');
        assert_eq!(initial_of('파'), Some(17));
        assert_eq!(initial_index('ㅍ'), Some(17));
        assert_eq!(initial_index('ㄳ'), None, "초성이 될 수 없는 자모");
        // 받침 나누기: 넘어간 초성 + 남은 받침이 원래 받침과 같은 소리다
        assert_eq!(JONG_SPLIT[21], (0, 11)); // ㅇ
        assert_eq!(JONG_SPLIT[9], (8, 0)); // ㄺ → ㄹ + ㄱ
        let (l, v, t) = hangul('닭').unwrap();
        let (rest, moved) = JONG_SPLIT[t as usize];
        assert_eq!(compose(l, v, rest as u32), '달');
        assert_eq!(INITIALS[moved as usize], 'ㄱ');
    }
}

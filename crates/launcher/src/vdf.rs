//! vdf.rs: Bộ phân tích cú pháp và thao tác file cấu hình Valve Data Format (VDF / KeyValues)
//! Chuyên dụng để can thiệp vào `loginusers.vdf` và `libraryfolders.vdf` của Steam.

use std::collections::BTreeMap;
use std::path::Path;
use thiserror::Error;

const MAX_VDF_SIZE: usize = 4 * 1024 * 1024; // 4 MiB safety limit
const MAX_NESTING_DEPTH: usize = 16;
const MAX_ENTRIES: usize = 10_000;

#[derive(Error, Debug)]
pub enum VdfError {
    #[error("VDF exceeds 4 MiB safety limit")]
    ExceedsSizeLimit,
    #[error("Unexpected end of VDF")]
    UnexpectedEof,
    #[error("Unterminated VDF string")]
    UnterminatedString,
    #[error("Unterminated VDF escape")]
    UnterminatedEscape,
    #[error("Expected VDF string")]
    ExpectedString,
    #[error("Expected VDF object")]
    ExpectedObject,
    #[error("Missing closing VDF brace")]
    MissingClosingBrace,
    #[error("Unexpected closing VDF brace")]
    UnexpectedClosingBrace,
    #[error("VDF entry limit exceeded")]
    EntryLimitExceeded,
    #[error("Empty VDF key")]
    EmptyKey,
    #[error("VDF nesting exceeds safety limit")]
    NestingExceeded,
    #[error("Missing Steam users section")]
    MissingUsersSection,
    #[error("users.vdf has no users section")]
    NoUsersSection,
    #[error("Lỗi I/O: {0}")]
    Io(#[from] std::io::Error),
}

#[derive(Debug, Clone, PartialEq)]
pub enum VdfValue {
    String(String),
    Object(BTreeMap<String, VdfValue>),
}

impl VdfValue {
    pub fn as_str(&self) -> Option<&str> {
        match self {
            VdfValue::String(s) => Some(s.as_str()),
            _ => None,
        }
    }

    pub fn as_object(&self) -> Option<&BTreeMap<String, VdfValue>> {
        match self {
            VdfValue::Object(m) => Some(m),
            _ => None,
        }
    }

    pub fn as_object_mut(&mut self) -> Option<&mut BTreeMap<String, VdfValue>> {
        match self {
            VdfValue::Object(m) => Some(m),
            _ => None,
        }
    }
}

pub struct VdfParser<'a> {
    input: &'a [u8],
    pos: usize,
    entry_count: usize,
}

impl<'a> VdfParser<'a> {
    pub fn new(input: &'a [u8]) -> Result<Self, VdfError> {
        if input.len() > MAX_VDF_SIZE {
            return Err(VdfError::ExceedsSizeLimit);
        }
        Ok(Self {
            input,
            pos: 0,
            entry_count: 0,
        })
    }

    fn skip_whitespace_and_comments(&mut self) {
        while self.pos < self.input.len() {
            let b = self.input[self.pos];
            if b.is_ascii_whitespace() {
                self.pos += 1;
            } else if b == b'/' && self.pos + 1 < self.input.len() && self.input[self.pos + 1] == b'/' {
                // Comment, skip to end of line
                self.pos += 2;
                while self.pos < self.input.len() && self.input[self.pos] != b'\n' {
                    self.pos += 1;
                }
            } else {
                break;
            }
        }
    }

    fn parse_string(&mut self) -> Result<String, VdfError> {
        self.skip_whitespace_and_comments();
        if self.pos >= self.input.len() {
            return Err(VdfError::UnexpectedEof);
        }

        if self.input[self.pos] == b'"' {
            self.pos += 1;
            let mut result = Vec::new();
            while self.pos < self.input.len() {
                let b = self.input[self.pos];
                if b == b'"' {
                    self.pos += 1;
                    return String::from_utf8(result).map_err(|_| VdfError::ExpectedString);
                } else if b == b'\\' {
                    self.pos += 1;
                    if self.pos >= self.input.len() {
                        return Err(VdfError::UnterminatedEscape);
                    }
                    match self.input[self.pos] {
                        b'\\' => result.push(b'\\'),
                        b'"' => result.push(b'"'),
                        b'n' => result.push(b'\n'),
                        b't' => result.push(b'\t'),
                        other => result.push(other),
                    }
                    self.pos += 1;
                } else {
                    result.push(b);
                    self.pos += 1;
                }
            }
            Err(VdfError::UnterminatedString)
        } else {
            // Bare string
            let start = self.pos;
            while self.pos < self.input.len()
                && !self.input[self.pos].is_ascii_whitespace()
                && self.input[self.pos] != b'{'
                && self.input[self.pos] != b'}'
            {
                self.pos += 1;
            }
            if start == self.pos {
                return Err(VdfError::ExpectedString);
            }
            String::from_utf8(self.input[start..self.pos].to_vec())
                .map_err(|_| VdfError::ExpectedString)
        }
    }

    pub fn parse_object(&mut self, depth: usize) -> Result<BTreeMap<String, VdfValue>, VdfError> {
        if depth > MAX_NESTING_DEPTH {
            return Err(VdfError::NestingExceeded);
        }

        let mut map = BTreeMap::new();
        loop {
            self.skip_whitespace_and_comments();
            if self.pos >= self.input.len() {
                if depth > 0 {
                    return Err(VdfError::MissingClosingBrace);
                }
                break;
            }

            if self.input[self.pos] == b'}' {
                if depth == 0 {
                    return Err(VdfError::UnexpectedClosingBrace);
                }
                self.pos += 1;
                break;
            }

            self.entry_count += 1;
            if self.entry_count > MAX_ENTRIES {
                return Err(VdfError::EntryLimitExceeded);
            }

            let key = self.parse_string()?;
            if key.is_empty() {
                return Err(VdfError::EmptyKey);
            }

            self.skip_whitespace_and_comments();
            if self.pos >= self.input.len() {
                return Err(VdfError::UnexpectedEof);
            }

            if self.input[self.pos] == b'{' {
                self.pos += 1;
                let child = self.parse_object(depth + 1)?;
                map.insert(key, VdfValue::Object(child));
            } else {
                let val = self.parse_string()?;
                map.insert(key, VdfValue::String(val));
            }
        }
        Ok(map)
    }

    pub fn parse(mut self) -> Result<BTreeMap<String, VdfValue>, VdfError> {
        self.parse_object(0)
    }
}

pub fn serialize_vdf(map: &BTreeMap<String, VdfValue>, indent: usize) -> String {
    let mut out = String::new();
    let prefix = "\t".repeat(indent);
    for (k, v) in map {
        match v {
            VdfValue::String(s) => {
                out.push_str(&format!("{}\"{}\"\t\t\"{}\"\n", prefix, k, s));
            }
            VdfValue::Object(child) => {
                out.push_str(&format!("{}\"{}\"\n{}{{\n", prefix, k, prefix));
                out.push_str(&serialize_vdf(child, indent + 1));
                out.push_str(&format!("{}}}\n", prefix));
            }
        }
    }
    out
}

/// Thao tác can thiệp trực tiếp vào file `loginusers.vdf` của Steam
pub fn patch_loginusers(
    vdf_path: &Path,
    target_account: &str,
    force_offline: bool,
) -> Result<(), VdfError> {
    let content = std::fs::read(vdf_path)?;
    let parser = VdfParser::new(&content)?;
    let mut root = parser.parse()?;

    let users_section = root
        .get_mut("users")
        .and_then(|v| v.as_object_mut())
        .ok_or(VdfError::MissingUsersSection)?;

    let mut matched = false;
    for (_steam_id, user_val) in users_section.iter_mut() {
        if let Some(user_obj) = user_val.as_object_mut() {
            let acc_name = user_obj
                .get("AccountName")
                .and_then(|v| v.as_str())
                .unwrap_or("");

            if acc_name.eq_ignore_ascii_case(target_account) {
                user_obj.insert("RememberPassword".into(), VdfValue::String("1".into()));
                user_obj.insert("MostRecent".into(), VdfValue::String("1".into()));
                if force_offline {
                    user_obj.insert("WantsOfflineMode".into(), VdfValue::String("1".into()));
                    user_obj.insert("SkipOfflineModeWarning".into(), VdfValue::String("1".into()));
                }
                matched = true;
            } else {
                user_obj.insert("MostRecent".into(), VdfValue::String("0".into()));
            }
        }
    }

    if !matched {
        return Err(VdfError::NoUsersSection);
    }

    let serialized = serialize_vdf(&root, 0);
    std::fs::write(vdf_path, serialized)?;
    Ok(())
}


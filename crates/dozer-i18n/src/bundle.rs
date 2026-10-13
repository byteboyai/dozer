//! 资源装载与校验:扁平 JSON 字符串表,按 owner/locale 组织。
//!
//! 校验必须发现:重复 JSON key(在反序列化覆盖前)、非字符串值、空字符串、
//! 错误括号及占位符不一致(spec §6.2)。这些错误在装载时即报,不进入运行时。

use std::collections::HashMap;
use std::fmt;

/// 资源装载错误。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BundleError {
    /// JSON 语法错误。
    Syntax(String),
    /// 顶层不是对象。
    NotAnObject,
    /// 重复 key(扁平表内同名)。
    DuplicateKey(String),
    /// 值不是字符串。
    NonStringValue(String),
    /// 空字符串翻译。
    EmptyValue(String),
    /// 模板括号错误或占位符名非法。
    BadTemplate { key: String, detail: String },
}

impl fmt::Display for BundleError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BundleError::Syntax(m) => write!(f, "JSON 语法错误: {m}"),
            BundleError::NotAnObject => write!(f, "资源顶层必须是字符串键值对象"),
            BundleError::DuplicateKey(k) => write!(f, "重复 key: {k}"),
            BundleError::NonStringValue(k) => write!(f, "值不是字符串: {k}"),
            BundleError::EmptyValue(k) => write!(f, "空翻译: {k}"),
            BundleError::BadTemplate { key, detail } => {
                write!(f, "模板非法({key}): {detail}")
            }
        }
    }
}

impl std::error::Error for BundleError {}

/// 一份语言包:扁平 key→模板 表。
#[derive(Debug, Clone, Default)]
pub struct ResourceBundle {
    entries: HashMap<String, String>,
}

impl ResourceBundle {
    /// 从扁平 JSON 文本装载并校验。
    pub fn from_json(json: &str) -> Result<Self, BundleError> {
        let value: serde_json::Value =
            serde_json::from_str(json).map_err(|e| BundleError::Syntax(e.to_string()))?;
        let obj = value.as_object().ok_or(BundleError::NotAnObject)?;

        // serde_json 默认保留最后一个重复 key,所以先用保留首个出现的原始扫描
        // 检测重复。用一个独立解析器统计每个 key 出现次数。
        let dup = find_duplicate_key(json);
        if let Some(k) = dup {
            return Err(BundleError::DuplicateKey(k));
        }

        let mut entries = HashMap::with_capacity(obj.len());
        for (k, v) in obj {
            let Some(s) = v.as_str() else {
                return Err(BundleError::NonStringValue(k.clone()));
            };
            if s.is_empty() {
                return Err(BundleError::EmptyValue(k.clone()));
            }
            validate_template(k, s)?;
            entries.insert(k.clone(), s.to_string());
        }
        Ok(ResourceBundle { entries })
    }

    pub fn get(&self, key: &str) -> Option<&str> {
        self.entries.get(key).map(String::as_str)
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn keys(&self) -> impl Iterator<Item = &str> {
        self.entries.keys().map(String::as_str)
    }
}

/// 校验模板:括号配对正确,占位符名合法,且非孤立花括号。
fn validate_template(key: &str, template: &str) -> Result<(), BundleError> {
    let mut i = 0;
    let bytes = template.as_bytes();
    while i < bytes.len() {
        match bytes[i] {
            b'{' if i + 1 < bytes.len() && bytes[i + 1] == b'{' => {
                i += 2;
            }
            b'}' if i + 1 < bytes.len() && bytes[i + 1] == b'}' => {
                i += 2;
            }
            b'{' => {
                let start = i + 1;
                let Some(rel) = template[start..].find('}') else {
                    return Err(BundleError::BadTemplate {
                        key: key.to_string(),
                        detail: "未闭合的 `{`".to_string(),
                    });
                };
                let end = start + rel;
                let name = &template[start..end];
                if !super::interp::is_valid_param_name(name) {
                    return Err(BundleError::BadTemplate {
                        key: key.to_string(),
                        detail: format!("非法占位符名 `{{{name}}}`"),
                    });
                }
                i = end + 1;
            }
            b'}' => {
                return Err(BundleError::BadTemplate {
                    key: key.to_string(),
                    detail: "未配对的 `}`".to_string(),
                });
            }
            _ => i += 1,
        }
    }
    Ok(())
}

/// 扫描原始 JSON 文本,返回第一个重复出现的顶层键(若有)。
///
/// 用一个极小的流式扫描而非 `serde_json` 的 Map,因为后者已丢弃重复信息。
/// 只处理顶层对象、字符串键、以及跳过值;足以发现扁平表的重复键。
fn find_duplicate_key(json: &str) -> Option<String> {
    let bytes = json.as_bytes();
    let mut i = 0;
    let n = bytes.len();
    // 跳过空白与顶层 `{`
    while i < n && bytes[i].is_ascii_whitespace() {
        i += 1;
    }
    if i >= n || bytes[i] != b'{' {
        return None;
    }
    i += 1;
    let mut seen = std::collections::HashSet::new();
    loop {
        while i < n && (bytes[i].is_ascii_whitespace() || bytes[i] == b',') {
            i += 1;
        }
        if i >= n || bytes[i] == b'}' {
            return None;
        }
        if bytes[i] != b'"' {
            return None; // 非字符串键:交给 serde 报语法错误
        }
        let (key, next) = read_json_string(json, i)?;
        if !seen.insert(key.clone()) {
            return Some(key);
        }
        i = next;
        // 跳过 `:` 与值
        while i < n && (bytes[i].is_ascii_whitespace() || bytes[i] == b':') {
            i += 1;
        }
        i = skip_json_value(json, i)?;
    }
}

/// 读取一个 JSON 字符串,返回(内容,结束引号后一位)。
fn read_json_string(json: &str, start: usize) -> Option<(String, usize)> {
    let bytes = json.as_bytes();
    let n = bytes.len();
    let mut i = start + 1;
    let mut out = String::new();
    while i < n {
        match bytes[i] {
            b'"' => return Some((out, i + 1)),
            b'\\' => {
                // 简化:转义序列交给 serde 处理语义,这里为检测重复只取原始片段。
                if i + 1 >= n {
                    return None;
                }
                out.push(bytes[i] as char);
                out.push(bytes[i + 1] as char);
                i += 2;
            }
            b => {
                out.push(b as char);
                i += 1;
            }
        }
    }
    None
}

/// 跳过一个 JSON 值,返回其结束后的位置。
fn skip_json_value(json: &str, start: usize) -> Option<usize> {
    let bytes = json.as_bytes();
    let n = bytes.len();
    let mut i = start;
    match bytes.get(i)? {
        b'"' => {
            let (_, next) = read_json_string(json, i)?;
            Some(next)
        }
        b'{' | b'[' => {
            let (open, close) = if bytes[i] == b'{' {
                (b'{', b'}')
            } else {
                (b'[', b']')
            };
            let mut depth = 0;
            let mut in_str = false;
            while i < n {
                let c = bytes[i];
                if in_str {
                    if c == b'\\' {
                        i += 2;
                        continue;
                    }
                    if c == b'"' {
                        in_str = false;
                    }
                } else if c == b'"' {
                    in_str = true;
                } else if c == open {
                    depth += 1;
                } else if c == close {
                    depth -= 1;
                    if depth == 0 {
                        return Some(i + 1);
                    }
                }
                i += 1;
            }
            None
        }
        _ => {
            while i < n && bytes[i] != b',' && bytes[i] != b'}' {
                i += 1;
            }
            Some(i)
        }
    }
}

/// 一个 owner 的完整资源集:多个 locale 的 bundle。
#[derive(Debug, Clone)]
pub struct ResourceSet {
    pub owner: String,
    pub default_locale: String,
    pub bundles: HashMap<String, ResourceBundle>,
}

impl ResourceSet {
    pub fn new(owner: impl Into<String>) -> Self {
        ResourceSet {
            owner: owner.into(),
            default_locale: super::DEFAULT_OWNER_LOCALE.to_string(),
            bundles: HashMap::new(),
        }
    }

    pub fn with_default_locale(mut self, locale: impl Into<String>) -> Self {
        self.default_locale = locale.into();
        self
    }

    pub fn insert_bundle(&mut self, locale: impl Into<String>, bundle: ResourceBundle) {
        self.bundles.insert(locale.into(), bundle);
    }

    pub fn bundle(&self, locale: &str) -> Option<&ResourceBundle> {
        self.bundles.get(locale)
    }
}

/// 校验两份语言包 key 集与占位符集一致(spec §6.1)。
/// 返回所有不一致项,空表示通过。
pub fn check_key_parity(
    reference: &ResourceBundle,
    candidate: &ResourceBundle,
) -> Vec<BundleError> {
    let mut errs = Vec::new();
    for key in reference.keys() {
        if candidate.get(key).is_none() {
            errs.push(BundleError::BadTemplate {
                key: key.to_string(),
                detail: "候选语言包缺少该 key".to_string(),
            });
        } else if let (Some(a), Some(b)) = (reference.get(key), candidate.get(key)) {
            let pa = placeholders(a);
            let pb = placeholders(b);
            if pa != pb {
                errs.push(BundleError::BadTemplate {
                    key: key.to_string(),
                    detail: format!("占位符集合不一致: {pa:?} vs {pb:?}"),
                });
            }
        }
    }
    errs
}

/// 提取模板中出现的占位符名集合(有序去重)。
pub fn placeholders(template: &str) -> Vec<String> {
    let mut out = Vec::new();
    let bytes = template.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'{' && i + 1 < bytes.len() && bytes[i + 1] == b'{' {
            i += 2;
            continue;
        }
        if bytes[i] == b'{' {
            let start = i + 1;
            if let Some(rel) = template[start..].find('}') {
                let name = &template[start..start + rel];
                let name = name.to_string();
                if !out.contains(&name) {
                    out.push(name);
                }
                i = start + rel + 1;
                continue;
            }
        }
        i += 1;
    }
    out
}

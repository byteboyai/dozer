//! 模板插值:扁平 JSON 字符串值,仅支持 `{name}`,`{{`/`}}` 表示字面花括号
//! (spec §6.2)。单次替换,不递归解释参数中的花括号,不支持 HTML/表达式。

use std::collections::BTreeMap;
use std::fmt;

/// 插值失败原因。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InterpolationError {
    /// 缺少参数。
    MissingArg(String),
    /// 占位符名非法(非 ASCII 字母/数字/下划线,或以数字开头)。
    BadName(String),
    /// 括号不配对。
    Unbalanced,
}

impl fmt::Display for InterpolationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            InterpolationError::MissingArg(n) => write!(f, "缺少参数 {n}"),
            InterpolationError::BadName(n) => write!(f, "非法占位符名 {n}"),
            InterpolationError::Unbalanced => write!(f, "括号不配对"),
        }
    }
}

impl std::error::Error for InterpolationError {}

/// 参数名规则:ASCII 字母/数字/下划线,不以数字开头(spec §6.2)。
/// 空串按非法处理(避免 `{}` 这种无意义占位)。
pub fn is_valid_param_name(name: &str) -> bool {
    let mut chars = name.chars();
    match chars.next() {
        None => false,
        Some(c) if c.is_ascii_alphabetic() || c == '_' => {
            chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
        }
        Some(_) => false,
    }
}

/// 对模板做一次插值。多余参数忽略;缺参数或非法模板返回错误。
pub fn interpolate(
    template: &str,
    args: &BTreeMap<String, String>,
) -> Result<String, InterpolationError> {
    let mut out = String::with_capacity(template.len());
    let bytes = template.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'{' if i + 1 < bytes.len() && bytes[i + 1] == b'{' => {
                out.push('{');
                i += 2;
            }
            b'}' if i + 1 < bytes.len() && bytes[i + 1] == b'}' => {
                out.push('}');
                i += 2;
            }
            b'{' => {
                let start = i + 1;
                let Some(rel) = template[start..].find('}') else {
                    return Err(InterpolationError::Unbalanced);
                };
                let end = start + rel;
                let name = &template[start..end];
                if !is_valid_param_name(name) {
                    return Err(InterpolationError::BadName(name.to_string()));
                }
                let Some(value) = args.get(name) else {
                    return Err(InterpolationError::MissingArg(name.to_string()));
                };
                out.push_str(value);
                i = end + 1;
            }
            b'}' => return Err(InterpolationError::Unbalanced),
            _ => {
                // 逐 char 推进,避免切断多字节 UTF-8。
                let ch = template[i..].chars().next().unwrap();
                out.push(ch);
                i += ch.len_utf8();
            }
        }
    }
    Ok(out)
}

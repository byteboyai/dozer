//! Dozer 多语言解析库(spec 2026-10-12-dozer-i18n-design §5/§6)。
//!
//! 纯查表与插值:无持久化、无 IO、无 iced、无网络。资源以扁平 JSON 字符串表
//! 提供,同名 key 在 `en`/`zh-CN` 各一份。本 crate 同时提供供 TS 解析器复用的
//! 一致性测试向量(`vectors/interpolation.json`)。
//!
//! 解析顺序(spec §6.1):当前 locale 的 key → owner default_locale 的 key →
//! 显式 fallback。任何一步得到空值/非法模板都视为该候选不可用,继续下一步;
//! 最终不得返回空白或 panic。

use std::collections::BTreeMap;
use std::collections::HashMap;

use dozer_model::LocalizedText;

mod bundle;
mod interp;
mod locale;

pub use bundle::{BundleError, ResourceBundle, ResourceSet, check_key_parity, placeholders};
pub use interp::{InterpolationError, interpolate, is_valid_param_name};
pub use locale::{SUPPORTED, normalize_locale, resolve_from_candidates};

/// owner 的默认语言表:官方 owner 默认 `en`(spec §6.1)。
pub const DEFAULT_OWNER_LOCALE: &str = "en";

/// 单次解析的诊断,便于调用方记录 owner/key/locale 而不泄露参数值。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolveDiagnostic {
    pub owner: String,
    pub key: Option<String>,
    pub locale: String,
    /// 未命中/不可用的原因,供日志去重使用。
    pub reason: ResolveMiss,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResolveMiss {
    /// 该 owner 无资源集(未加载)。
    NoResources,
    /// 当前与默认 locale 都缺 key。
    MissingKey,
    /// 找到模板但缺参数或模板非法。
    Unrenderable,
}

/// `LocalizedText` 的解析结果;`text` 必非空(失败时是内置安全文案)。
#[derive(Debug, Clone)]
pub struct Resolved {
    pub text: String,
    pub diagnostic: Option<ResolveDiagnostic>,
}

/// 解析器:持有按 owner 组织的资源集,按 locale 查表并插值。
#[derive(Debug, Default)]
pub struct Resolver {
    sets: HashMap<String, ResourceSet>,
}

impl Resolver {
    pub fn new() -> Self {
        Resolver::default()
    }

    /// 注册(或替换)一个 owner 的资源集。
    pub fn insert(&mut self, set: ResourceSet) {
        self.sets.insert(set.owner.clone(), set);
    }

    /// 解析一条文案。`text` 恒非空;命中失败时返回内置安全文案并带诊断。
    pub fn resolve(&self, text: &LocalizedText, locale: &str) -> Resolved {
        let Some(key) = text.key.as_deref() else {
            // 无 key:直接用 fallback,若 fallback 为空则用安全文案。
            return self.render_fallback_only(text);
        };

        let Some(set) = self.sets.get(&text.owner) else {
            return self.render_fallback(text, ResolveMiss::NoResources);
        };

        let candidates = [locale, set.default_locale.as_str()];
        let mut saw_template = false;
        for cand in candidates {
            let Some(bundle) = set.bundles.get(cand) else {
                continue;
            };
            let Some(template) = bundle.get(key) else {
                continue;
            };
            saw_template = true;
            match interp::interpolate(template, &text.args) {
                Ok(rendered) if !rendered.trim().is_empty() => {
                    return Resolved {
                        text: rendered,
                        diagnostic: None,
                    };
                }
                _ => continue,
            }
        }

        let reason = if saw_template {
            ResolveMiss::Unrenderable
        } else {
            ResolveMiss::MissingKey
        };
        self.render_fallback(text, reason)
    }

    fn render_fallback_only(&self, text: &LocalizedText) -> Resolved {
        if !text.fallback.trim().is_empty() {
            return Resolved {
                text: text.fallback.clone(),
                diagnostic: None,
            };
        }
        Resolved {
            text: SAFE_FALLBACK.to_string(),
            diagnostic: Some(ResolveDiagnostic {
                owner: text.owner.clone(),
                key: None,
                locale: DEFAULT_OWNER_LOCALE.to_string(),
                reason: ResolveMiss::MissingKey,
            }),
        }
    }

    fn render_fallback(&self, text: &LocalizedText, reason: ResolveMiss) -> Resolved {
        // 官方 fallback 是英文安全文案;尝试对 fallback 本身做插值,
        // 缺参数时仍回落内置文案,避免带 `{name}` 的字面量泄漏到界面。
        let text_out = if !text.fallback.trim().is_empty() {
            match interp::interpolate(&text.fallback, &text.args) {
                Ok(s) if !s.trim().is_empty() => s,
                _ => SAFE_FALLBACK.to_string(),
            }
        } else {
            SAFE_FALLBACK.to_string()
        };
        Resolved {
            text: text_out,
            diagnostic: Some(ResolveDiagnostic {
                owner: text.owner.clone(),
                key: text.key.clone(),
                locale: DEFAULT_OWNER_LOCALE.to_string(),
                reason,
            }),
        }
    }
}

/// 所有候选都不可用时的内置安全文案(不依赖任何资源,不 panic)。
pub const SAFE_FALLBACK: &str = "Something went wrong";

/// 构建一个 `Resolver`,从 `(owner, locale) -> 扁平 JSON 文本` 装载。
/// 便捷函数,供测试与调用方批量装载。
pub fn resolver_from_json(
    entries: impl IntoIterator<Item = (String, String, String)>,
) -> Result<Resolver, BundleError> {
    let mut grouped: BTreeMap<String, ResourceSet> = BTreeMap::new();
    for (owner, locale, json) in entries {
        let bundle = ResourceBundle::from_json(&json)?;
        grouped
            .entry(owner.clone())
            .or_insert_with(|| ResourceSet::new(owner))
            .insert_bundle(locale, bundle);
    }
    let mut r = Resolver::new();
    for (_, set) in grouped {
        r.insert(set);
    }
    Ok(r)
}
